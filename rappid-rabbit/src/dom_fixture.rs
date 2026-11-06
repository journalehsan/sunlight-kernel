//! Opt-in SunJS host for controlled local fixtures only.
//!
//! Navigation never calls this module. One-shot evaluation and persistent local
//! sessions own a context and copy-on-write DOM lease. Native
//! callbacks only mutate owned data and request invalidation, never paint.

#![forbid(unsafe_code)]

use std::{cell::RefCell, collections::BTreeMap, rc::Rc, sync::Arc};

use golden_fish::{Document, Node, NodeId};
use sunlight_js::{Context, Error, ErrorKind, Handle, HostRoot, JsResult, Limits, Value};

use crate::render::{DocumentRenderState, TextMeasurer};

const MAX_DOM_NODES: usize = 4096;
const MAX_TEXT_BYTES: usize = 256 * 1024;
const MAX_HOST_WORK: usize = 256 * 1024;
const MAX_MUTATIONS: usize = 256;

/// Script errors do not roll back earlier writes. Owners must honor the paint
/// notification even when evaluation failed; the scene and patch are ready.
#[derive(Debug)]
#[must_use = "inspect repaint_requested even when evaluation failed"]
pub struct FixtureRun {
    pub evaluation: JsResult<()>,
    pub repaint_requested: bool,
}

/// Execute explicitly supplied, trusted fixture source and flush invalidation
/// once, including on exceptional exit. No script discovery, I/O, or scheduling.
/// The owner is exclusively borrowed through layout, so no worker can commit
/// during the turn. Later completions must also match `dom_revision`.
pub fn run_fixture_script(
    render: &mut DocumentRenderState,
    source: &str,
    source_name: &str,
    measurer: &dyn TextMeasurer,
) -> FixtureRun {
    let host = match DomHost::new(render.dom.clone()) {
        Ok(host) => Rc::new(RefCell::new(host)),
        Err(error) => {
            return FixtureRun {
                evaluation: Err(error),
                repaint_requested: false,
            }
        }
    };
    let mut js = Context::new(Limits::default());
    let evaluation = install(&mut js, &host)
        .and_then(|()| js.eval(source, source_name).map(|_| ()))
        .map_err(|mut error| {
            // An object thrown by JS belongs to the context being discarded.
            // Preserve diagnostics and primitive payloads, never stale handles.
            if matches!(error.thrown, Some(Value::Object(_))) {
                error.thrown = None;
            }
            error
        });
    // Callbacks and cached handles are discarded before publishing the DOM.
    drop(js);
    let host = host.borrow();
    if host.dirty {
        render.rebuild_for_fixture_dom(host.dom.clone(), measurer);
    }
    for (id, control) in &host.forms.controls {
        render.patch_control(
            *id,
            control,
            host.forms.focused_control == Some(*id),
            measurer,
        );
    }
    FixtureRun {
        evaluation,
        repaint_requested: host.dirty,
    }
}

struct Listener {
    id: usize,
    node: NodeId,
    kind: String,
    callback: HostRoot,
}

struct DomHost {
    dom: Arc<Document>,
    wrappers: BTreeMap<NodeId, HostRoot>,
    nodes: BTreeMap<Handle, NodeId>,
    element: Option<HostRoot>,
    listeners: Vec<Listener>,
    next_listener: usize,
    next_task: usize,
    dispatch_depth: usize,
    tasks: std::collections::VecDeque<HostRoot>,
    focus_value: Option<String>,
    forms: crate::form::FormState,
    dirty: bool,
    text_budget: usize,
    work: usize,
    mutations: usize,
}

impl DomHost {
    fn new(dom: Arc<Document>) -> JsResult<Self> {
        if dom.node_count() > MAX_DOM_NODES {
            return Err(limit("fixture DOM node limit"));
        }
        // Include detached subtrees left by earlier turns, not just live text.
        let bytes: usize = (0..dom.node_count())
            .filter_map(|id| match dom.get(id) {
                Some(Node::Text { content } | Node::Comment { content }) => Some(content.len()),
                Some(Node::Element { attributes, .. }) => Some(
                    attributes
                        .iter()
                        .map(|a| a.name().len() + a.value().len())
                        .sum(),
                ),
                _ => None,
            })
            .sum();
        let text_budget = MAX_TEXT_BYTES
            .checked_sub(bytes)
            .ok_or_else(|| limit("fixture DOM text limit"))?;
        let mut forms = crate::form::FormState::new();
        forms.build_from_dom(&dom);
        Ok(Self {
            dom,
            wrappers: BTreeMap::new(),
            nodes: BTreeMap::new(),
            element: None,
            listeners: Vec::new(),
            next_listener: 0,
            next_task: 0,
            dispatch_depth: 0,
            tasks: std::collections::VecDeque::new(),
            focus_value: None,
            forms,
            dirty: false,
            text_budget,
            work: MAX_HOST_WORK,
            mutations: MAX_MUTATIONS,
        })
    }

    fn charge(&mut self) -> JsResult<()> {
        // Bound native traversal work independently of interpreter fuel.
        self.charge_work(self.dom.node_count())
    }

    fn charge_work(&mut self, amount: usize) -> JsResult<()> {
        self.work = self
            .work
            .checked_sub(amount)
            .ok_or_else(|| limit("fixture DOM work limit"))?;
        Ok(())
    }

    fn receiver(&self, receiver: &Value) -> JsResult<NodeId> {
        match receiver {
            Value::Object(handle) => self.nodes.get(handle).copied(),
            _ => None,
        }
        .ok_or_else(|| Error::new(ErrorKind::Type, "invalid element receiver"))
    }
}

fn limit(message: &str) -> Error {
    Error::new(ErrorKind::Limit, message)
}

// Golden Fish stores UTF-8. This minimal bridge deliberately accepts strings
// only and rejects lone surrogates, without lossy replacement or JS coercion.
fn string_argument(value: Option<&Value>, host: &mut DomHost) -> JsResult<String> {
    match value {
        Some(Value::String(units)) => {
            host.charge_work(units.len())?;
            String::from_utf16(units)
                .map_err(|_| Error::new(ErrorKind::Type, "DOM strings must be valid Unicode"))
        }
        _ => Err(Error::new(ErrorKind::Type, "DOM host requires a string")),
    }
}

fn install(js: &mut Context, host: &Rc<RefCell<DomHost>>) -> JsResult<()> {
    let element = js.create_object(Some(js.object_prototype()))?;
    let read_host = host.clone();
    let get = js.native_function(move |_, receiver, _| {
        let mut host = read_host.borrow_mut();
        let node = host.receiver(&receiver)?;
        host.charge()?;
        let text = host
            .dom
            .element_text_content(node)
            .ok_or_else(|| Error::new(ErrorKind::Type, "invalid DOM element"))?;
        host.charge_work(text.len())?;
        Ok(Value::string(&text))
    })?;
    let write_host = host.clone();
    let set = js.native_function(move |_, receiver, args| {
        let mut host = write_host.borrow_mut();
        let node = host.receiver(&receiver)?;
        host.charge()?;
        // String validation never invokes JS. No borrow crosses JS execution.
        let text = string_argument(args.first(), &mut host)?;
        if host.mutations == 0 || host.dom.node_count() >= MAX_DOM_NODES {
            return Err(limit("fixture DOM mutation/node limit"));
        }
        host.text_budget = host
            .text_budget
            .checked_sub(text.len())
            .ok_or_else(|| limit("fixture DOM text allocation limit"))?;
        host.mutations -= 1;
        let changed = Arc::make_mut(&mut host.dom)
            .set_element_text_content(node, &text)
            .ok_or_else(|| Error::new(ErrorKind::Type, "invalid DOM element"))?;
        host.dirty |= changed;
        Ok(Value::Undefined)
    })?;
    js.define_accessor(element, "textContent", Some(get), Some(set))?;

    host.borrow_mut().element = Some(js.register_root(Value::Object(element))?);
    install_controls(js, host, element)?;
    let document = js.create_object(Some(js.object_prototype()))?;
    let lookup_host = host.clone();
    let lookup = js.native_function_with_roots(
        vec![Value::Object(element), Value::Object(document)],
        move |js, receiver, args| {
            if receiver != Value::Object(document) {
                return Err(Error::new(ErrorKind::Type, "invalid document receiver"));
            }
            let node = {
                let mut host = lookup_host.borrow_mut();
                let id = string_argument(args.first(), &mut host)?;
                let comparison_work = host.dom.node_count().saturating_mul(id.len().max(1));
                host.charge_work(comparison_work)?;
                host.dom.get_element_by_id(&id)
            };
            let Some(node) = node else {
                return Ok(Value::Null);
            };
            wrapper(js, &lookup_host, node)
        },
    )?;
    js.define_data(document, "getElementById", lookup, false)?;
    let query_host = host.clone();
    let query =
        js.native_function_with_roots(vec![Value::Object(document)], move |js, receiver, args| {
            if receiver != Value::Object(document) {
                return Err(type_error("invalid document receiver"));
            }
            let node = {
                let mut h = query_host.borrow_mut();
                h.charge()?;
                let selector = string_argument(args.first(), &mut h)?;
                h.dom.query_selector(&selector).map_err(type_error)?
            };
            match node {
                Some(node) => wrapper(js, &query_host, node),
                None => Ok(Value::Null),
            }
        })?;
    js.define_data(document, "querySelector", query, false)?;
    let focus_host = host.clone();
    let active = js.native_function(move |js, _, _| {
        let node = focus_host.borrow().forms.focused_control;
        match node {
            Some(node) => wrapper(js, &focus_host, node.0 as usize),
            None => Ok(Value::Null),
        }
    })?;
    js.define_accessor(document, "activeElement", Some(active), None)?;
    js.set_global("document", Value::Object(document))?;
    js.eval(include_str!("fixture_events.js"), "<fixture-events>")?;
    let window = js.eval("globalThis", "<fixture-host>")?;
    js.set_global("window", window)
}

#[cfg(test)]
mod gc_tests {
    use super::*;

    #[test]
    fn cached_and_detached_wrappers_survive_collection() {
        let dom =
            golden_fish::parse_html("<div id='parent'><span id='child'>Old</span></div>").unwrap();
        let host = Rc::new(RefCell::new(DomHost::new(Arc::new(dom)).unwrap()));
        let mut js = Context::default();
        install(&mut js, &host).unwrap();
        js.set_gc_threshold(0);
        js.register_native("gc", |js, _, _| {
            js.collect_garbage()?;
            Ok(Value::Undefined)
        })
        .unwrap();
        assert_eq!(js.eval("let child=document.getElementById('child');gc();child===document.getElementById('child')", "identity").unwrap(), Value::Boolean(true));
        // Remove the script root; the identity map alone must keep the wrapper.
        js.eval("child=null", "release").unwrap();
        let original = host.borrow().wrappers.values().next().unwrap().value();
        for _ in 0..4 {
            js.collect_garbage().unwrap();
        }
        assert_eq!(
            js.eval("document.getElementById('child')", "cached")
                .unwrap(),
            original
        );
        js.eval("child=document.getElementById('child');document.getElementById('parent').textContent='New';gc();", "detach").unwrap();
        assert_eq!(
            js.eval(
                "child.textContent==='Old' && document.getElementById('child')===null",
                "survivor"
            )
            .unwrap(),
            Value::Boolean(true)
        );
        js.close().unwrap();
        assert!(js.get_property(original, "textContent").is_err());
    }
}

fn type_error(message: &str) -> Error {
    Error::new(ErrorKind::Type, message)
}
fn control_id(node: NodeId) -> sunlight_ui::widgets::DocumentNodeId {
    sunlight_ui::widgets::DocumentNodeId(node as u64)
}
fn wrapper(js: &mut Context, host: &Rc<RefCell<DomHost>>, node: NodeId) -> JsResult<Value> {
    if let Some(root) = host.borrow().wrappers.get(&node) {
        return Ok(root.value());
    }
    let proto = match host.borrow().element.as_ref().unwrap().value() {
        Value::Object(h) => h,
        _ => unreachable!(),
    };
    let handle = js.create_object(Some(proto))?;
    let root = js.register_root(Value::Object(handle))?;
    let mut h = host.borrow_mut();
    h.wrappers.insert(node, root);
    h.nodes.insert(handle, node);
    Ok(Value::Object(handle))
}

fn attribute(h: &mut DomHost, node: NodeId, name: &str, value: Option<&str>) -> JsResult<()> {
    h.charge()?;
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(type_error("unsupported attribute name"));
    }
    if h.mutations == 0 {
        return Err(limit("fixture DOM mutation limit"));
    }
    h.mutations -= 1;
    h.text_budget = h
        .text_budget
        .checked_sub(value.map_or(0, str::len))
        .ok_or_else(|| limit("fixture attribute allocation limit"))?;
    let changed = Arc::make_mut(&mut h.dom)
        .set_attribute(node, name, value)
        .ok_or_else(|| type_error("invalid element"))?;
    h.dirty |= changed;
    // Re-read reflected metadata without overwriting dirty runtime values.
    let old = h.forms.clone();
    h.forms.build_from_dom(&h.dom);
    h.forms.focused_control = old.focused_control;
    for (id, control) in &mut h.forms.controls {
        if let (
            crate::form::FormControlState::Text { state, .. },
            Some(crate::form::FormControlState::Text { state: before, .. }),
        ) = (&mut *control, old.controls.get(id))
        {
            if before.dirty {
                *state = before.clone();
            }
        }
        if name != "checked" {
            if let Some(before) = old.controls.get(id) {
                if let (
                    crate::form::FormControlState::Button {
                        kind: crate::form::FormControlKind::Checkbox { checked },
                        ..
                    },
                    crate::form::FormControlKind::Checkbox { checked: prior },
                ) = (control, before.kind())
                {
                    *checked = *prior;
                }
            }
        }
    }
    Ok(())
}

fn install_controls(
    js: &mut Context,
    host: &Rc<RefCell<DomHost>>,
    element: Handle,
) -> JsResult<()> {
    for method in [
        "getAttribute",
        "hasAttribute",
        "setAttribute",
        "removeAttribute",
    ] {
        let host = host.clone();
        let function = js.native_function(move |_, receiver, args| {
            let mut h = host.borrow_mut();
            let node = h.receiver(&receiver)?;
            let name = string_argument(args.first(), &mut h)?.to_ascii_lowercase();
            h.charge()?;
            match method {
                "getAttribute" => Ok(h
                    .dom
                    .get_attribute(node, &name)
                    .map(Value::string)
                    .unwrap_or(Value::Null)),
                "hasAttribute" => Ok(Value::Boolean(h.dom.get_attribute(node, &name).is_some())),
                _ => {
                    let value = if method == "setAttribute" {
                        Some(string_argument(args.get(1), &mut h)?)
                    } else {
                        None
                    };
                    attribute(&mut h, node, &name, value.as_deref())?;
                    Ok(Value::Undefined)
                }
            }
        })?;
        js.define_data(element, method, function, false)?;
    }
    for property in [
        "value",
        "checked",
        "disabled",
        "type",
        "name",
        "className",
        "form",
        "parentElement",
    ] {
        let host_get = host.clone();
        let get = js.native_function(move |js, receiver, _| {
            let node;
            let related;
            {
                let mut h = host_get.borrow_mut();
                node = h.receiver(&receiver)?;
                h.charge()?;
                let control = h.forms.controls.get(&control_id(node));
                match property {
                    "value" => return Ok(Value::string(control.map_or("", |c| c.current_value()))),
                    "checked" => {
                        return Ok(Value::Boolean(control.is_some_and(|c| {
                            matches!(
                                c.kind(),
                                crate::form::FormControlKind::Checkbox { checked: true }
                            )
                        })))
                    }
                    "disabled" => {
                        return Ok(Value::Boolean(
                            h.dom.get_attribute(node, "disabled").is_some(),
                        ))
                    }
                    "form" => {
                        related = control.and_then(|c| c.form_owner()).map(|id| id.0 as usize)
                    }
                    "parentElement" => {
                        related = h
                            .dom
                            .parent(node)
                            .filter(|id| h.dom.get(*id).is_some_and(Node::is_element))
                    }
                    _ => {
                        return Ok(Value::string(
                            h.dom
                                .get_attribute(
                                    node,
                                    if property == "className" {
                                        "class"
                                    } else {
                                        property
                                    },
                                )
                                .unwrap_or(if property == "type" { "text" } else { "" }),
                        ))
                    }
                }
            }
            match related {
                Some(node) => wrapper(js, &host_get, node),
                None => Ok(Value::Null),
            }
        })?;
        let host_set = host.clone();
        let set = js.native_function(move |_, receiver, args| {
            let mut h = host_set.borrow_mut();
            let node = h.receiver(&receiver)?;
            if property == "value" {
                let value = string_argument(args.first(), &mut h)?;
                if value.len() > 4096 {
                    return Err(limit("control value limit"));
                }
                h.charge_work(value.len())?;
                if let Some(crate::form::FormControlState::Text { state, .. }) =
                    h.forms.controls.get_mut(&control_id(node))
                {
                    state.cursor_position = value.len();
                    state.current_value = value.clone();
                    state.dirty = true;
                    if h.forms.focused_control == Some(control_id(node)) {
                        h.focus_value = Some(value);
                    }
                    h.dirty = true;
                } else {
                    return Err(type_error("value requires text input"));
                }
            } else if property == "checked" {
                let Some(Value::Boolean(value)) = args.first() else {
                    return Err(type_error("checked requires boolean"));
                };
                if let Some(crate::form::FormControlState::Button {
                    kind: crate::form::FormControlKind::Checkbox { checked },
                    ..
                }) = h.forms.controls.get_mut(&control_id(node))
                {
                    *checked = *value;
                    h.dirty = true;
                }
            } else if property == "disabled" {
                let Some(Value::Boolean(value)) = args.first() else {
                    return Err(type_error("disabled requires boolean"));
                };
                attribute(
                    &mut h,
                    node,
                    "disabled",
                    if *value { Some("") } else { None },
                )?;
            } else if property == "form" || property == "parentElement" {
                return Err(type_error("read-only property"));
            } else {
                let value = string_argument(args.first(), &mut h)?;
                attribute(
                    &mut h,
                    node,
                    if property == "className" {
                        "class"
                    } else {
                        property
                    },
                    Some(&value),
                )?;
            }
            Ok(Value::Undefined)
        })?;
        js.define_accessor(element, property, Some(get), Some(set))?;
    }
    for method in [
        "addEventListener",
        "removeEventListener",
        "dispatchEvent",
        "click",
        "focus",
        "blur",
        "requestSubmit",
    ] {
        let host = host.clone();
        let function = js.native_function(move |js, receiver, args| {
            let node = host.borrow().receiver(&receiver)?;
            match method {
                "addEventListener" | "removeEventListener" => {
                    let mut h = host.borrow_mut();
                    let kind = string_argument(args.first(), &mut h)?;
                    if !matches!(
                        args.get(2),
                        None | Some(Value::Undefined | Value::Boolean(false))
                    ) {
                        return Err(type_error("capture/options unsupported"));
                    }
                    let callback = args
                        .get(1)
                        .cloned()
                        .ok_or_else(|| type_error("missing callback"))?;
                    if !js.is_callable(&callback) {
                        return Err(type_error("listener must be a function"));
                    }
                    let found = h.listeners.iter().position(|l| {
                        l.node == node && l.kind == kind && l.callback.value() == callback
                    });
                    if method == "removeEventListener" {
                        if let Some(i) = found {
                            h.listeners.remove(i);
                        }
                    } else if found.is_none() {
                        if h.listeners.len() >= 128 {
                            return Err(limit("listener limit"));
                        }
                        let id = h.next_listener;
                        h.next_listener += 1;
                        h.listeners.push(Listener {
                            id,
                            node,
                            kind,
                            callback: js.register_root(callback)?,
                        });
                    }
                    Ok(Value::Undefined)
                }
                "dispatchEvent" => dispatch(
                    js,
                    &host,
                    node,
                    args.first()
                        .cloned()
                        .ok_or_else(|| type_error("missing event"))?,
                ),
                "focus" => {
                    focus(js, &host, Some(node))?;
                    Ok(Value::Undefined)
                }
                "blur" => {
                    if host.borrow().forms.focused_control == Some(control_id(node)) {
                        focus(js, &host, None)?;
                    }
                    Ok(Value::Undefined)
                }
                "click" => {
                    activate(js, &host, node)?;
                    Ok(Value::Undefined)
                }
                _ => {
                    if host.borrow().dom.tag_name(node) != Some("form") {
                        return Err(type_error("requestSubmit requires form"));
                    }
                    emit(js, &host, node, "submit", true)?;
                    Ok(Value::Undefined)
                }
            }
        })?;
        js.define_data(element, method, function, false)?;
    }
    let task_host = host.clone();
    js.register_native("fixtureAuthorize", move |js, _, args| {
        if args.len() != 1 || !js.is_callable(&args[0]) {
            return Err(type_error("authorization accepts only a callback"));
        }
        let mut h = task_host.borrow_mut();
        if h.tasks.len() >= 8 {
            return Err(limit("host task limit"));
        }
        h.next_task = h
            .next_task
            .checked_add(1)
            .ok_or_else(|| limit("host request handle limit"))?;
        h.tasks.push_back(js.register_root(args[0].clone())?);
        Ok(Value::Number(h.next_task as f64))
    })?;
    Ok(())
}

fn dispatch(
    js: &mut Context,
    host: &Rc<RefCell<DomHost>>,
    node: NodeId,
    event: Value,
) -> JsResult<Value> {
    let _root = js.register_root(event.clone())?;
    let type_value = js.get_property(event.clone(), "type")?;
    let kind = {
        let mut h = host.borrow_mut();
        string_argument(Some(&type_value), &mut h)?
    };
    if js.get_property(event.clone(), "_dispatching")? == Value::Boolean(true) {
        return Err(type_error("event already dispatching"));
    }
    let bubbles = js.get_property(event.clone(), "bubbles")? == Value::Boolean(true);
    let mut path = vec![node];
    {
        let mut h = host.borrow_mut();
        h.charge()?;
        if h.dispatch_depth >= 16 {
            return Err(limit("event dispatch depth limit"));
        }
        if bubbles {
            let mut next = h.dom.parent(node);
            while let Some(id) = next {
                if h.dom.get(id).is_some_and(Node::is_element) {
                    path.push(id);
                }
                next = h.dom.parent(id);
            }
        }
        h.dispatch_depth += 1;
    }
    let result = (|| {
        let target = wrapper(js, host, node)?;
        js.set_property(event.clone(), "target", target)?;
        js.set_property(event.clone(), "_dispatching", Value::Boolean(true))?;
        js.set_property(event.clone(), "_stop", Value::Boolean(false))?;
        for current in path {
            let receiver = wrapper(js, host, current)?;
            js.set_property(event.clone(), "currentTarget", receiver.clone())?;
            let listeners: Vec<_> = host
                .borrow()
                .listeners
                .iter()
                .filter(|l| l.node == current && l.kind == kind)
                .map(|l| (l.id, l.callback.clone()))
                .collect();
            for (id, callback) in listeners {
                if host.borrow().listeners.iter().any(|l| l.id == id) {
                    js.call(
                        callback.value(),
                        receiver.clone(),
                        std::slice::from_ref(&event),
                    )?;
                }
            }
            if js.get_property(event.clone(), "_stop")? == Value::Boolean(true) {
                break;
            }
        }
        Ok(Value::Boolean(
            js.get_property(event.clone(), "defaultPrevented")? != Value::Boolean(true),
        ))
    })();
    host.borrow_mut().dispatch_depth -= 1;
    let cleanup = js
        .set_property(event.clone(), "currentTarget", Value::Null)
        .and_then(|()| js.set_property(event, "_dispatching", Value::Boolean(false)));
    result.and_then(|value| cleanup.map(|()| value))
}
fn emit(
    js: &mut Context,
    host: &Rc<RefCell<DomHost>>,
    node: NodeId,
    kind: &str,
    bubbles: bool,
) -> JsResult<bool> {
    // Event names originate only in this module, never user text.
    let cancelable = matches!(kind, "click" | "submit");
    let event = js.eval(
        &format!("Event('{kind}', {{bubbles:{bubbles},cancelable:{cancelable}}})"),
        "<host-event>",
    )?;
    Ok(dispatch(js, host, node, event)? == Value::Boolean(true))
}
fn focusable(h: &DomHost, node: NodeId) -> bool {
    if h.dom.get_attribute(node, "tabindex") == Some("-1") {
        return false;
    }
    if h.dom.tag_name(node) == Some("a") {
        return h.dom.get_attribute(node, "href").is_some();
    }
    h.forms.controls.get(&control_id(node)).is_some_and(|c| {
        !c.is_disabled() && !matches!(c.kind(), crate::form::FormControlKind::HiddenInput)
    })
}

fn focus(js: &mut Context, host: &Rc<RefCell<DomHost>>, next: Option<NodeId>) -> JsResult<()> {
    let old = host.borrow().forms.focused_control;
    if old == next.map(control_id) {
        return Ok(());
    }
    if let Some(node) = next {
        if !focusable(&host.borrow(), node) {
            return Ok(());
        }
    }
    host.borrow_mut().forms.focused_control = None;
    if let Some(old) = old {
        let changed = {
            let h = host.borrow();
            h.focus_value.as_ref().is_some_and(|value| {
                h.forms
                    .controls
                    .get(&old)
                    .is_some_and(|c| c.current_value() != value)
            })
        };
        if changed {
            emit(js, host, old.0 as usize, "change", true)?;
        }
        emit(js, host, old.0 as usize, "blur", false)?;
    }
    host.borrow_mut().forms.focused_control = next.map(control_id);
    let baseline = {
        let h = host.borrow();
        next.and_then(|node| h.forms.controls.get(&control_id(node)))
            .filter(|c| c.kind().is_text())
            .map(|c| c.current_value_owned())
    };
    host.borrow_mut().focus_value = baseline;
    host.borrow_mut().dirty = true;
    if let Some(next) = next {
        emit(js, host, next, "focus", false)?;
    }
    Ok(())
}
fn activate(js: &mut Context, host: &Rc<RefCell<DomHost>>, node: NodeId) -> JsResult<()> {
    let label_target = {
        let h = host.borrow();
        if h.dom.tag_name(node) == Some("label") {
            h.dom
                .get_attribute(node, "for")
                .and_then(|id| h.dom.get_element_by_id(id))
                .filter(|id| h.forms.controls.contains_key(&control_id(*id)))
        } else {
            None
        }
    };
    if let Some(target) = label_target {
        return activate(js, host, target);
    }
    if host
        .borrow()
        .forms
        .controls
        .get(&control_id(node))
        .is_some_and(|c| c.is_disabled())
    {
        return Ok(());
    }
    focus(js, host, Some(node))?;
    // Checkbox pre-activation is visible to click listeners and rolled back on cancellation.
    let checked = {
        let mut h = host.borrow_mut();
        if let Some(crate::form::FormControlState::Button {
            kind: crate::form::FormControlKind::Checkbox { checked },
            ..
        }) = h.forms.controls.get_mut(&control_id(node))
        {
            let old = *checked;
            *checked = !old;
            Some(old)
        } else {
            None
        }
    };
    let allowed = emit(js, host, node, "click", true)?;
    if let Some(old) = checked {
        if allowed {
            emit(js, host, node, "input", true)?;
            emit(js, host, node, "change", true)?;
        } else if let Some(crate::form::FormControlState::Button {
            kind: crate::form::FormControlKind::Checkbox { checked },
            ..
        }) = host.borrow_mut().forms.controls.get_mut(&control_id(node))
        {
            *checked = old;
        }
        host.borrow_mut().dirty = true;
    }
    if allowed {
        let form = host
            .borrow()
            .forms
            .controls
            .get(&control_id(node))
            .filter(|c| c.kind().is_submit())
            .and_then(|c| c.form_owner());
        if let Some(form) = form {
            emit(js, host, form.0 as usize, "submit", true)?;
        }
    }
    Ok(())
}

/// Persistent capability for one explicitly selected local document. Drop on navigation.
/// No Debug implementation: control values and pending callbacks are private.
pub struct FixtureSession {
    js: Context,
    host: Rc<RefCell<DomHost>>,
    generation: u64,
    active: bool,
}
impl FixtureSession {
    pub fn new(render: &DocumentRenderState) -> JsResult<Self> {
        let host = Rc::new(RefCell::new(DomHost::new(render.dom.clone())?));
        let mut js = Context::default();
        install(&mut js, &host)?;
        Ok(Self {
            js,
            host,
            generation: render.document_id,
            active: true,
        })
    }
    fn begin(&mut self, render: &DocumentRenderState) -> JsResult<()> {
        if !self.active
            || self.generation != render.document_id
            || !Arc::ptr_eq(&self.host.borrow().dom, &render.dom)
        {
            self.clear();
            return Err(type_error("stale fixture generation"));
        }
        let mut h = self.host.borrow_mut();
        h.work = MAX_HOST_WORK;
        h.mutations = MAX_MUTATIONS;
        Ok(())
    }
    fn flush(&mut self, render: &mut DocumentRenderState, measure: &dyn TextMeasurer) {
        let mut h = self.host.borrow_mut();
        if h.dirty {
            render.rebuild_for_fixture_dom(h.dom.clone(), measure);
            h.dirty = false;
        }
        for (id, control) in &h.forms.controls {
            render.patch_control(*id, control, h.forms.focused_control == Some(*id), measure);
        }
    }
    pub fn evaluate(
        &mut self,
        render: &mut DocumentRenderState,
        source: &str,
        measure: &dyn TextMeasurer,
    ) -> JsResult<()> {
        self.begin(render)?;
        let result = self.js.eval(source, "<controlled-fixture>").map(|_| ());
        self.flush(render, measure);
        result
    }
    pub fn click(
        &mut self,
        render: &mut DocumentRenderState,
        node: NodeId,
        measure: &dyn TextMeasurer,
    ) -> JsResult<()> {
        self.begin(render)?;
        let result = self.js.host_turn(|js| activate(js, &self.host, node));
        self.flush(render, measure);
        result
    }
    /// Deliver at most one queued completion per explicit UI turn. No network or wait.
    pub fn complete(
        &mut self,
        render: &mut DocumentRenderState,
        success: bool,
        measure: &dyn TextMeasurer,
    ) -> JsResult<bool> {
        self.begin(render)?;
        let task = self.host.borrow_mut().tasks.pop_front();
        let Some(task) = task else {
            return Ok(false);
        };
        let result = self
            .js
            .host_turn(|js| js.call(task.value(), Value::Undefined, &[Value::Boolean(success)]));
        self.flush(render, measure);
        result.map(|_| true)
    }
    /// Cancel outstanding test authorization requests without invoking callbacks.
    pub fn cancel_pending(&mut self) {
        self.host.borrow_mut().tasks.clear();
    }
    pub fn stress_gc(&mut self) {
        self.js.set_gc_threshold(0);
    }
    pub fn collect(&mut self) -> JsResult<()> {
        self.js.collect_garbage().map(|_| ())
    }
    fn clear(&mut self) {
        self.active = false;
        let mut h = self.host.borrow_mut();
        h.focus_value = None;
        for control in h.forms.controls.values_mut() {
            if let crate::form::FormControlState::Text {
                state,
                initial_value,
                ..
            } = control
            {
                state.current_value.clear();
                initial_value.clear();
            }
        }
        h.tasks.clear();
        h.listeners.clear();
        h.wrappers.clear();
        h.nodes.clear();
        h.element = None;
    }
}
impl Drop for FixtureSession {
    fn drop(&mut self) {
        self.clear();
        let _ = self.js.close();
    }
}

/// Logical keyboard actions; indices remain private UTF-8 boundaries, never JS offsets.
#[derive(Clone, Copy)]
pub enum FixtureKey {
    Text(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Tab(bool),
    Enter,
    Space,
    Escape,
    Up,
    Down,
}
/// Modifier snapshot supplied by the native physical-event stream.
#[derive(Clone, Copy, Default)]
pub struct KeyModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}
impl FixtureSession {
    pub fn key(
        &mut self,
        render: &mut DocumentRenderState,
        key: FixtureKey,
        measure: &dyn TextMeasurer,
    ) -> JsResult<()> {
        self.key_with_modifiers(render, key, KeyModifiers::default(), measure)
    }

    pub fn key_with_modifiers(
        &mut self,
        render: &mut DocumentRenderState,
        key: FixtureKey,
        modifiers: KeyModifiers,
        measure: &dyn TextMeasurer,
    ) -> JsResult<()> {
        self.begin(render)?;
        let host = self.host.clone();
        let result = self.js.host_turn(|js| {
            let focused = host.borrow().forms.focused_control;
            if let Some(id) = focused {
                if host
                    .borrow()
                    .forms
                    .controls
                    .get(&id)
                    .is_some_and(|c| c.is_disabled())
                {
                    return Ok(());
                }
                let event = js.eval(
                    "Event('keydown',{bubbles:true,cancelable:true})",
                    "<keyboard>",
                )?;
                let name = match key {
                    FixtureKey::Text(c) => c.to_string(),
                    FixtureKey::Backspace => "Backspace".into(),
                    FixtureKey::Delete => "Delete".into(),
                    FixtureKey::Left => "ArrowLeft".into(),
                    FixtureKey::Right => "ArrowRight".into(),
                    FixtureKey::Home => "Home".into(),
                    FixtureKey::End => "End".into(),
                    FixtureKey::Tab(_) => "Tab".into(),
                    FixtureKey::Enter => "Enter".into(),
                    FixtureKey::Space => " ".into(),
                    FixtureKey::Escape => "Escape".into(),
                    FixtureKey::Up => "ArrowUp".into(),
                    FixtureKey::Down => "ArrowDown".into(),
                };
                js.set_property(event.clone(), "key", Value::string(&name))?;
                js.set_property(
                    event.clone(),
                    "shiftKey",
                    Value::Boolean(modifiers.shift || matches!(key, FixtureKey::Tab(true))),
                )?;
                for (name, value) in [
                    ("ctrlKey", modifiers.ctrl),
                    ("altKey", modifiers.alt),
                    ("metaKey", modifiers.meta),
                ] {
                    js.set_property(event.clone(), name, Value::Boolean(value))?;
                }
                if dispatch(js, &host, id.0 as usize, event)? == Value::Boolean(false) {
                    return Ok(());
                }
            }
            if let FixtureKey::Tab(back) = key {
                let next = {
                    let h = host.borrow();
                    let mut stack = vec![h.dom.root()];
                    let mut ordered = Vec::new();
                    while let Some(node) = stack.pop() {
                        if focusable(&h, node) {
                            ordered.push(node);
                        }
                        stack.extend(h.dom.children(node).iter().rev().copied());
                    }
                    if ordered.len() > 256 {
                        return Err(limit("focus traversal limit"));
                    }
                    if ordered.is_empty() {
                        None
                    } else {
                        let position = focused
                            .and_then(|id| ordered.iter().position(|node| *node == id.0 as usize));
                        let index = match position {
                            Some(i) if back => (i + ordered.len() - 1) % ordered.len(),
                            Some(i) => (i + 1) % ordered.len(),
                            None if back => ordered.len() - 1,
                            None => 0,
                        };
                        Some(ordered[index])
                    }
                };
                return focus(js, &host, next);
            }
            let Some(id) = focused else {
                return Ok(());
            };
            if matches!(key, FixtureKey::Enter) {
                if host.borrow().dom.tag_name(id.0 as usize) == Some("a") {
                    return activate(js, &host, id.0 as usize);
                }
                let owner = host
                    .borrow()
                    .forms
                    .controls
                    .get(&id)
                    .and_then(|c| c.form_owner());
                if let Some(owner) = owner {
                    emit(js, &host, owner.0 as usize, "submit", true)?;
                }
                return Ok(());
            }
            if matches!(key, FixtureKey::Space)
                && host
                    .borrow()
                    .forms
                    .controls
                    .get(&id)
                    .is_some_and(|c| !c.kind().is_text())
            {
                return activate(js, &host, id.0 as usize);
            }
            let (changed, input) = {
                let mut h = host.borrow_mut();
                match key {
                    FixtureKey::Text(c) if !c.is_control() => (h.forms.insert_char(c), true),
                    FixtureKey::Space => (h.forms.insert_char(' '), true),
                    FixtureKey::Backspace => (h.forms.backspace(), true),
                    FixtureKey::Delete => (h.forms.delete_forward(), true),
                    FixtureKey::Left => (h.forms.move_cursor_left(), false),
                    FixtureKey::Right => (h.forms.move_cursor_right(), false),
                    FixtureKey::Home => (h.forms.move_cursor_home(), false),
                    FixtureKey::End => (h.forms.move_cursor_end(), false),
                    _ => (false, false),
                }
            };
            if changed {
                host.borrow_mut().dirty = true;
                if input {
                    emit(js, &host, id.0 as usize, "input", true)?;
                }
            }
            Ok(())
        });
        self.flush(render, measure);
        result
    }
}

#[cfg(test)]
mod session_lifecycle_tests {
    use super::*;
    use crate::account_fixture::{document, Measure};
    #[test]
    fn listener_cycles_and_pending_closures_release_on_teardown() {
        let mut render = document(1);
        let mut session = FixtureSession::new(&render).unwrap();
        session.stress_gc();
        session.evaluate(&mut render, "(function(){const node=document.getElementById('email');node.addEventListener('cycle',function(){return node;});fixtureAuthorize(function(){return node;});})();", &Measure).unwrap();
        session.collect().unwrap();
        let before = session.js.collect_garbage().unwrap().live_objects;
        let weak = Rc::downgrade(&session.host);
        session.clear();
        let stats = session.js.collect_garbage().unwrap();
        assert!(stats.live_objects < before);
        assert!(session.host.borrow().tasks.is_empty());
        assert!(session.host.borrow().listeners.is_empty());
        drop(session);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn cancel_and_limits_are_deterministic() {
        let mut render = document(1);
        let mut session = FixtureSession::new(&render).unwrap();
        session.stress_gc();
        session
            .evaluate(
                &mut render,
                "fixtureAuthorize(function(){throw 'canceled callback';});",
                &Measure,
            )
            .unwrap();
        session.cancel_pending();
        assert!(!session.complete(&mut render, true, &Measure).unwrap());
        assert!(session
            .evaluate(
                &mut render,
                "for(let i=0;i<9;i=i+1)fixtureAuthorize(function(){});",
                &Measure
            )
            .is_err());
        session.cancel_pending();
        assert!(session.evaluate(&mut render, "const email=document.getElementById('email');for(let i=0;i<129;i=i+1)email.addEventListener('x',function(){});", &Measure).is_err());
    }
}
