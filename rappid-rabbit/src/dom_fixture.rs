//! Opt-in, synchronous SunJS host for controlled local fixtures only.
//!
//! Navigation never calls this module. Each invocation owns a fresh context and
//! a copy-on-write DOM lease; no JS handles survive the invocation. Native
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
    FixtureRun {
        evaluation,
        repaint_requested: host.dirty,
    }
}

struct DomHost {
    dom: Arc<Document>,
    wrappers: BTreeMap<NodeId, HostRoot>,
    nodes: BTreeMap<Handle, NodeId>,
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
                _ => None,
            })
            .sum();
        let text_budget = MAX_TEXT_BYTES
            .checked_sub(bytes)
            .ok_or_else(|| limit("fixture DOM text limit"))?;
        Ok(Self {
            dom,
            wrappers: BTreeMap::new(),
            nodes: BTreeMap::new(),
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
            if let Some(handle) = lookup_host.borrow().wrappers.get(&node) {
                return Ok(handle.value());
            }
            // Allocate before publishing either direction of the identity map.
            let handle = js.create_object(Some(element))?;
            let mut host = lookup_host.borrow_mut();
            host.wrappers
                .insert(node, js.register_root(Value::Object(handle))?);
            host.nodes.insert(handle, node);
            Ok(Value::Object(handle))
        },
    )?;
    js.define_data(document, "getElementById", lookup, false)?;
    js.set_global("document", Value::Object(document))?;
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
