mod descriptors;
mod gc;
mod host;
mod intrinsics;
mod objects;
mod operators;
use crate::{ast::*, lexer, parser, Error, ErrorKind, Handle, JsResult, Value};
use gc::{Arena, CellType, Frame};
pub use gc::{GcStats, HostRoot};
use std::{
    collections::BTreeMap,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT_CONTEXT: AtomicU64 = AtomicU64::new(1);
type Native = Rc<dyn Fn(&mut Context, Value, &[Value]) -> JsResult<Value>>;
type Key = Vec<u16>;
/// Partial descriptor: omitted fields retain existing attributes on redefinition.
/// `Some(Value::Undefined)` clears an accessor callback.
#[derive(Clone, Debug, Default)]
pub struct PropertyDescriptor {
    pub value: Option<Value>,
    pub writable: Option<bool>,
    pub get: Option<Value>,
    pub set: Option<Value>,
    pub enumerable: Option<bool>,
    pub configurable: Option<bool>,
}
#[derive(Clone)]
enum Property {
    GlobalBinding(String),
    Data(Value, bool, bool, bool),
    Accessor(Option<Value>, Option<Value>, bool, bool),
}
#[derive(Clone)]
struct Function {
    params: Vec<String>,
    body: Rc<Vec<Stmt>>,
    env: usize,
    lexical_this: Option<Value>,
    source: Rc<Source>,
}
#[derive(Clone)]
enum ObjectKind {
    Ordinary,
    Array(u32),
    Function(Function),
    Native(Native, Vec<Value>),
}
struct Object {
    prototype: Option<Handle>,
    props: BTreeMap<Key, Property>,
    kind: ObjectKind,
}
#[derive(Clone)]
struct Binding {
    value: Option<Value>,
    constant: bool,
}
#[derive(Clone)]
struct Env {
    parent: Option<usize>,
    bindings: BTreeMap<String, Binding>,
}
#[derive(Default)]
struct Source {
    name: String,
    text: String,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub fuel: u64,
    pub call_depth: usize,
    pub arena_entries: usize,
    pub source_bytes: usize,
    pub string_units: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: 100_000,
            call_depth: 64,
            arena_entries: 100_000,
            source_bytes: 256 * 1024,
            string_units: 1024 * 1024,
        }
    }
}
/// A tracing heap. Host-held handles must be pinned across collection points.
pub struct Context {
    id: u64,
    limits: Limits,
    fuel: u64,
    depth: usize,
    running: usize,
    closed: bool,
    entries: usize,
    global: Handle,
    objects: Arena<Object>,
    envs: Arena<Env>,
    frames: Vec<Frame>,
    roots: Vec<std::rc::Weak<Value>>,
    gc_threshold: usize,
    gc_allocations: usize,
    collections: usize,
    source: Rc<Source>,
    object_proto: Handle,
    array_proto: Handle,
    function_proto: Handle,
}
enum Completion {
    Normal(Option<Value>),
    Return(Value),
    Break(Option<Value>),
    Continue(Option<Value>),
}
enum Reference {
    Unresolved(String),
    Binding(usize, String),
    Property(Value, Value),
}
impl Default for Context {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}
impl Context {
    pub fn new(limits: Limits) -> Self {
        let id = NEXT_CONTEXT.fetch_add(1, Ordering::Relaxed);
        let mut c = Self {
            id,
            limits,
            fuel: limits.fuel,
            depth: 0,
            running: 0,
            closed: false,
            entries: 13,
            global: Handle(id, 3, 0),
            objects: Arena::new(CellType::Object),
            frames: Vec::new(),
            roots: Vec::new(),
            gc_threshold: 256,
            gc_allocations: 0,
            collections: 0,
            envs: Arena::singleton(
                CellType::Environment,
                Env {
                    parent: None,
                    bindings: BTreeMap::new(),
                },
            ),
            source: Rc::new(Source::default()),
            object_proto: Handle(id, 0, 0),
            array_proto: Handle(id, 1, 0),
            function_proto: Handle(id, 2, 0),
        };
        for proto in [
            None,
            Some(c.object_proto),
            Some(c.object_proto),
            Some(c.object_proto),
        ] {
            c.objects.push(Object {
                prototype: proto,
                props: BTreeMap::new(),
                kind: ObjectKind::Ordinary,
            });
        }
        for (n, v) in [
            ("globalThis", Value::Object(c.global)),
            ("undefined", Value::Undefined),
            ("NaN", Value::Number(f64::NAN)),
            ("Infinity", Value::Number(f64::INFINITY)),
        ] {
            c.envs[0].bindings.insert(
                n.into(),
                Binding {
                    value: Some(v),
                    constant: n != "globalThis",
                },
            );
            c.objects[c.global.1].props.insert(
                n.encode_utf16().collect(),
                Property::GlobalBinding(n.into()),
            );
        }
        // Bootstrap is bounded and independent of the caller's execution limits.
        c.limits = Limits::default();
        c.install_intrinsics().expect("intrinsic bootstrap");
        c.limits = limits;
        c.fuel = limits.fuel;
        c
    }
    pub fn object_prototype(&self) -> Handle {
        self.object_proto
    }
    pub fn arena_usage(&self) -> (usize, usize) {
        (self.objects.live(), self.envs.live())
    }
    /// Invalidates all handles. A closed context cannot be reused.
    pub fn close(&mut self) -> JsResult<()> {
        if self.running != 0 || self.depth != 0 {
            return Err(Error::new(
                ErrorKind::Type,
                "cannot close an active context",
            ));
        }
        self.roots.clear();
        self.objects.clear();
        self.envs.clear();
        self.source = Rc::new(Source::default());
        self.closed = true;
        Ok(())
    }
    fn check(&self) -> JsResult<()> {
        if self.closed {
            Err(Error::new(ErrorKind::Reference, "context is closed"))
        } else {
            Ok(())
        }
    }
    fn charge(&mut self) -> JsResult<()> {
        self.check()?;
        if self.fuel == 0 {
            return Err(Error::new(ErrorKind::Limit, "execution fuel exhausted"));
        }
        if self.gc_allocations >= self.gc_threshold {
            self.collect_garbage()?;
        }
        self.fuel -= 1;
        Ok(())
    }
    fn capacity(&self) -> JsResult<()> {
        self.check()?;
        if self.entries >= self.limits.arena_entries {
            Err(Error::new(ErrorKind::Limit, "context arena limit"))
        } else {
            Ok(())
        }
    }
    fn object(&self, h: Handle) -> JsResult<&Object> {
        self.check()?;
        if h.0 != self.id {
            return Err(Error::new(ErrorKind::Reference, "foreign context handle"));
        }
        self.objects
            .get_handle(h.1, h.2)
            .ok_or_else(|| Error::new(ErrorKind::Reference, "stale object handle"))
    }
    fn validate(&self, v: &Value) -> JsResult<()> {
        if let Value::Object(h) = v {
            self.object(*h)?;
        }
        if let Value::String(s) = v {
            if s.len() > self.limits.string_units {
                return Err(Error::new(ErrorKind::Limit, "string size limit"));
            }
        }
        Ok(())
    }
    fn alloc(&mut self, kind: ObjectKind, prototype: Option<Handle>) -> JsResult<Handle> {
        self.capacity()?;
        if let Some(p) = prototype {
            self.object(p)?;
        }
        let index = self.objects.push(Object {
            kind,
            prototype,
            props: BTreeMap::new(),
        });
        let h = Handle(self.id, index, self.objects.generation(index));
        self.keep(Value::Object(h));
        self.gc_allocations += 1;
        self.entries += 1;
        if let ObjectKind::Array(len) = self.objects[h.1].kind {
            self.capacity()?;
            self.entries += 1;
            self.objects[h.1].props.insert(
                "length".encode_utf16().collect(),
                Property::Data(Value::Number(len as f64), true, false, false),
            );
        }
        Ok(h)
    }
    pub fn eval(&mut self, source: &str, name: &str) -> JsResult<Value> {
        self.check()?;
        if source.len() > self.limits.source_bytes {
            return Err(Error::new(ErrorKind::Limit, "source size limit"));
        }
        if self.running >= self.limits.call_depth {
            return Err(Error::new(ErrorKind::Limit, "evaluation reentry limit"));
        }
        let body = parser::parse(source, name)?;
        if self.running == 0 {
            self.fuel = self.limits.fuel;
        }
        let old = self.source.clone();
        self.source = Rc::new(Source {
            name: name.into(),
            text: source.into(),
        });
        self.frames.push(Frame::default());
        self.keep_env(0);
        self.running += 1;
        let r = (|| {
            self.prepare(0, &body)?;
            match self.list(&body, 0, Value::Object(self.global))? {
                Completion::Normal(v) => Ok(v.unwrap_or(Value::Undefined)),
                _ => Err(Error::new(
                    ErrorKind::Syntax,
                    "invalid top-level completion",
                )),
            }
        })();
        let r = r.map_err(|e| self.locate(e, 0));
        self.running -= 1;
        self.source = old;
        self.frames.pop();
        self.keep_result(&r);
        r
    }
    fn locate(&self, e: Error, pos: usize) -> Error {
        if e.source.is_empty() {
            lexer::located(e, &self.source.text, &self.source.name, pos)
        } else {
            e
        }
    }
    fn env(&mut self, parent: usize) -> JsResult<usize> {
        self.capacity()?;
        let e = self.envs.push(Env {
            parent: Some(parent),
            bindings: BTreeMap::new(),
        });
        self.keep_env(e);
        self.gc_allocations += 1;
        self.entries += 1;
        Ok(e)
    }
    fn binding(&self, mut e: usize, n: &str) -> JsResult<usize> {
        loop {
            if self.envs[e].bindings.contains_key(n) {
                return Ok(e);
            }
            match self.envs[e].parent {
                Some(p) => e = p,
                None => {
                    return Err(Error::new(
                        ErrorKind::Reference,
                        format!("{n} is not defined"),
                    ))
                }
            }
        }
    }
    fn read(&self, e: usize, n: &str) -> JsResult<Value> {
        self.envs[e].bindings[n].value.clone().ok_or_else(|| {
            Error::new(
                ErrorKind::Reference,
                format!("{n} is in temporal dead zone"),
            )
        })
    }
    fn prepare(&mut self, e: usize, body: &[Stmt]) -> JsResult<()> {
        for s in body {
            let names: Vec<&String> = match &s.kind {
                S::Decl(_, ds) => ds.iter().map(|(n, _)| n).collect(),
                S::Function(n, _) => vec![n],
                _ => Vec::new(),
            };
            for n in names {
                if self.envs[e].bindings.contains_key(n) {
                    return Err(Error::new(
                        ErrorKind::Syntax,
                        format!("duplicate binding {n}"),
                    ));
                }
            }
        }
        for s in body {
            match &s.kind {
                S::Decl(constant, ds) => {
                    for (n, _) in ds {
                        self.declare(e, n, *constant)?;
                    }
                }
                S::Function(n, _) => self.declare(e, n, false)?,
                _ => {}
            }
        }
        for s in body {
            if let S::Function(n, f) = &s.kind {
                let v = self.expression(f, e, Value::Undefined)?;
                self.envs[e].bindings.get_mut(n).unwrap().value = Some(v);
                if e == 0 {
                    let key: Key = n.encode_utf16().collect();
                    if !self.objects[self.global.1].props.contains_key(&key) {
                        self.capacity()?;
                        self.entries += 1;
                    }
                    self.objects[self.global.1]
                        .props
                        .insert(key, Property::GlobalBinding(n.clone()));
                }
            }
        }
        Ok(())
    }
    fn declare(&mut self, e: usize, n: &str, constant: bool) -> JsResult<()> {
        if self.envs[e].bindings.contains_key(n) {
            return Err(Error::new(
                ErrorKind::Syntax,
                format!("duplicate binding {n}"),
            ));
        }
        self.capacity()?;
        self.entries += 1;
        self.envs[e].bindings.insert(
            n.into(),
            Binding {
                value: None,
                constant,
            },
        );
        Ok(())
    }
    fn list(&mut self, body: &[Stmt], e: usize, this: Value) -> JsResult<Completion> {
        self.frames.push(Frame::default());
        self.keep_env(e);
        self.keep_ref(&this);
        let r = self.list_inner(body, e, this);
        self.frames.pop();
        self.keep_completion(&r);
        r
    }
    fn list_inner(&mut self, body: &[Stmt], e: usize, this: Value) -> JsResult<Completion> {
        let mut last = None;
        for s in body {
            self.reset_frame(e, &this, last.as_ref());
            match self.statement(s, e, this.clone())? {
                Completion::Normal(v) => {
                    if v.is_some() {
                        last = v;
                    }
                }
                Completion::Break(v) => return Ok(Completion::Break(v.or(last))),
                Completion::Continue(v) => return Ok(Completion::Continue(v.or(last))),
                c => return Ok(c),
            }
        }
        Ok(Completion::Normal(last))
    }
    fn statement(&mut self, s: &Stmt, e: usize, this: Value) -> JsResult<Completion> {
        self.frames.push(Frame::default());
        self.keep_env(e);
        self.keep_ref(&this);
        let r = self.statement_inner(s, e, this);
        self.frames.pop();
        self.keep_completion(&r);
        r
    }
    fn statement_inner(&mut self, s: &Stmt, e: usize, this: Value) -> JsResult<Completion> {
        self.charge().map_err(|x| self.locate(x, s.pos))?;
        let r = (|| {
            Ok(match &s.kind {
                S::Empty | S::Function(_, _) => Completion::Normal(None),
                S::Expr(x) => Completion::Normal(Some(self.expression(x, e, this)?)),
                S::Decl(_, ds) => {
                    for (n, x) in ds {
                        let v = if let Some(x) = x {
                            self.expression(x, e, this.clone())?
                        } else {
                            Value::Undefined
                        };
                        self.envs[e].bindings.get_mut(n).unwrap().value = Some(v);
                    }
                    Completion::Normal(None)
                }
                S::Block(body) => {
                    let b = self.env(e)?;
                    self.prepare(b, body)?;
                    self.list(body, b, this)?
                }
                S::If(c, y, n) => {
                    let completion = if self.expression(c, e, this.clone())?.truthy() {
                        self.statement(y, e, this)?
                    } else if let Some(n) = n {
                        self.statement(n, e, this)?
                    } else {
                        Completion::Normal(None)
                    };
                    match completion {
                        Completion::Normal(v) => {
                            Completion::Normal(Some(v.unwrap_or(Value::Undefined)))
                        }
                        Completion::Break(v) => {
                            Completion::Break(Some(v.unwrap_or(Value::Undefined)))
                        }
                        Completion::Continue(v) => {
                            Completion::Continue(Some(v.unwrap_or(Value::Undefined)))
                        }
                        r => r,
                    }
                }
                S::While(c, b) => {
                    let mut last = Some(Value::Undefined);
                    loop {
                        self.reset_frame(e, &this, last.as_ref());
                        if !self.expression(c, e, this.clone())?.truthy() {
                            break;
                        }
                        self.charge()?;
                        match self.statement(b, e, this.clone())? {
                            Completion::Break(v) => {
                                if v.is_some() {
                                    last = v;
                                }
                                break;
                            }
                            Completion::Continue(v) => {
                                if v.is_some() {
                                    last = v;
                                }
                            }
                            Completion::Normal(v) => {
                                if v.is_some() {
                                    last = v;
                                }
                            }
                            r => return Ok(r),
                        }
                    }
                    Completion::Normal(last)
                }
                S::For(init, test, step, b) => {
                    let mut env = self.env(e)?;
                    let mut per_iteration = false;
                    if let Some(init) = init {
                        if let S::Decl(constant, ds) = &init.kind {
                            for (n, _) in ds {
                                self.declare(env, n, *constant)?;
                            }
                            per_iteration = !*constant;
                        }
                        self.statement(init, env, this.clone())?;
                    }
                    if per_iteration {
                        env = self.clone_iteration(env)?;
                    }
                    let mut last = Some(Value::Undefined);
                    loop {
                        self.reset_frame(e, &this, last.as_ref());
                        self.keep_env(env);
                        self.charge()?;
                        if let Some(test) = test {
                            if !self.expression(test, env, this.clone())?.truthy() {
                                break;
                            }
                        }
                        match self.statement(b, env, this.clone())? {
                            Completion::Break(v) => {
                                if v.is_some() {
                                    last = v;
                                }
                                break;
                            }
                            Completion::Continue(v) => {
                                if v.is_some() {
                                    last = v;
                                }
                            }
                            Completion::Normal(v) => {
                                if v.is_some() {
                                    last = v;
                                }
                            }
                            r => return Ok(r),
                        }
                        if per_iteration {
                            env = self.clone_iteration(env)?;
                        }
                        if let Some(step) = step {
                            self.expression(step, env, this.clone())?;
                        }
                    }
                    Completion::Normal(last)
                }
                S::Return(x) => Completion::Return(if let Some(x) = x {
                    self.expression(x, e, this)?
                } else {
                    Value::Undefined
                }),
                S::Break => Completion::Break(None),
                S::Continue => Completion::Continue(None),
                S::Throw(x) => {
                    let v = self.expression(x, e, this)?;
                    let mut err = Error::new(ErrorKind::Exception, v.display());
                    err.thrown = Some(v);
                    return Err(err);
                }
            })
        })();
        r.map_err(|x| self.locate(x, s.pos))
    }
    fn clone_iteration(&mut self, e: usize) -> JsResult<usize> {
        self.capacity()?;
        let count = self.envs[e].bindings.len() + 1;
        if self.entries.saturating_add(count) > self.limits.arena_entries {
            return Err(Error::new(ErrorKind::Limit, "context arena limit"));
        }
        self.entries += count;
        let n = self.envs.push(self.envs[e].clone());
        self.keep_env(n);
        self.gc_allocations += 1;
        Ok(n)
    }
    fn reference(&mut self, x: &Expr, e: usize, this: Value) -> JsResult<Reference> {
        match &x.kind {
            E::Id(n) => Ok(match self.binding(e, n) {
                Ok(e) => Reference::Binding(e, n.clone()),
                Err(_) => Reference::Unresolved(n.clone()),
            }),
            E::Group(x) => self.reference(x, e, this),
            E::Member(base, key) => {
                let b = self.expression(base, e, this.clone())?;
                let k = self.expression(key, e, this)?;
                Ok(Reference::Property(b, k))
            }
            _ => Err(Error::new(ErrorKind::Syntax, "invalid assignment target")),
        }
    }
    fn deref(&mut self, r: &Reference) -> JsResult<Value> {
        match r {
            Reference::Unresolved(n) => Err(Error::new(
                ErrorKind::Reference,
                format!("{n} is not defined"),
            )),
            Reference::Binding(e, n) => self.read(*e, n),
            Reference::Property(v, k) => {
                if matches!(v, Value::Null | Value::Undefined) {
                    return Err(Error::new(ErrorKind::Type, "cannot access null/undefined"));
                }
                let key = self.key(k.clone())?;
                self.get(v.clone(), &key)
            }
        }
    }
    fn expression(&mut self, x: &Expr, e: usize, this: Value) -> JsResult<Value> {
        self.frames.push(Frame::default());
        self.keep_env(e);
        self.keep_ref(&this);
        let r = self.expression_inner(x, e, this);
        self.frames.pop();
        self.keep_result(&r);
        r
    }
    fn expression_inner(&mut self, x: &Expr, e: usize, this: Value) -> JsResult<Value> {
        self.charge().map_err(|err| self.locate(err, x.pos))?;
        let r = (|| match &x.kind {
            E::Literal(v) => {
                self.validate(v)?;
                Ok(v.clone())
            }
            E::This => Ok(this),
            E::Id(n) => self.read(self.binding(e, n)?, n),
            E::Group(x) => self.expression(x, e, this),
            E::Unary(op, x) => {
                let v = self.expression(x, e, this)?;
                Ok(match op.as_str() {
                    "!" => Value::Boolean(!v.truthy()),
                    "-" => Value::Number(-self.number(v)?),
                    _ => Value::Number(self.number(v)?),
                })
            }
            E::Binary(op, a, b) => {
                let a = self.expression(a, e, this.clone())?;
                if (op == "&&" && !a.truthy())
                    || (op == "||" && a.truthy())
                    || (op == "??" && !matches!(a, Value::Null | Value::Undefined))
                {
                    return Ok(a);
                }
                let b = self.expression(b, e, this)?;
                if matches!(op.as_str(), "&&" | "||" | "??") {
                    return Ok(b);
                }
                self.binary(op, a, b)
            }
            E::Assign(a, b) => {
                let r = self.reference(a, e, this.clone())?;
                let v = self.expression(b, e, this)?;
                match r {
                    Reference::Unresolved(n) => {
                        return Err(Error::new(
                            ErrorKind::Reference,
                            format!("{n} is not defined"),
                        ))
                    }
                    Reference::Binding(env, n) => {
                        let binding = self.envs[env].bindings.get_mut(&n).unwrap();
                        if binding.value.is_none() {
                            return Err(Error::new(
                                ErrorKind::Reference,
                                "assignment in temporal dead zone",
                            ));
                        }
                        if binding.constant {
                            return Err(Error::new(ErrorKind::Type, "assignment to const"));
                        }
                        binding.value = Some(v.clone());
                    }
                    Reference::Property(base, key) => {
                        if matches!(base, Value::Null | Value::Undefined) {
                            return Err(Error::new(
                                ErrorKind::Type,
                                "cannot assign property of null/undefined",
                            ));
                        }
                        let key = self.key(key)?;
                        self.put(base, key, v.clone())?;
                    }
                }
                Ok(v)
            }
            E::Conditional(c, y, n) => {
                if self.expression(c, e, this.clone())?.truthy() {
                    self.expression(y, e, this)
                } else {
                    self.expression(n, e, this)
                }
            }
            E::Member(_, _) => {
                let r = self.reference(x, e, this)?;
                self.deref(&r)
            }
            E::Call(f, args) => {
                let (fun, receiver) = if Self::member(f) {
                    let r = self.reference(f, e, this.clone())?;
                    let v = self.deref(&r)?;
                    let receiver = match r {
                        Reference::Property(v, _) => v,
                        _ => Value::Undefined,
                    };
                    (v, receiver)
                } else {
                    (self.expression(f, e, this.clone())?, Value::Undefined)
                };
                let mut vs = Vec::new();
                for arg in args {
                    vs.push(self.expression(arg, e, this.clone())?);
                }
                self.call(fun, receiver, &vs)
            }
            E::Function(name, params, body, kind) => {
                let env = if name.is_some() { self.env(e)? } else { e };
                let f = Function {
                    params: params.clone(),
                    body: Rc::new(body.clone()),
                    env,
                    lexical_this: if *kind == FunctionKind::Arrow {
                        Some(this)
                    } else {
                        None
                    },
                    source: self.source.clone(),
                };
                let h = self.alloc(ObjectKind::Function(f), Some(self.function_proto))?;
                if *kind == FunctionKind::Ordinary {
                    let p = self.create_object(Some(self.object_proto))?;
                    self.define_property(
                        h,
                        "prototype",
                        PropertyDescriptor {
                            value: Some(Value::Object(p)),
                            writable: Some(true),
                            ..PropertyDescriptor::default()
                        },
                    )?;
                    self.define_property(
                        p,
                        "constructor",
                        PropertyDescriptor {
                            value: Some(Value::Object(h)),
                            writable: Some(true),
                            configurable: Some(true),
                            ..PropertyDescriptor::default()
                        },
                    )?;
                }
                if let Some(n) = name {
                    self.capacity()?;
                    self.entries += 1;
                    self.envs[env].bindings.insert(
                        n.clone(),
                        Binding {
                            constant: true,
                            value: Some(Value::Object(h)),
                        },
                    );
                }
                Ok(Value::Object(h))
            }
            E::Object(props) => {
                let h = self.create_object(Some(self.object_proto))?;
                for (k, x) in props {
                    let v = self.expression(x, e, this.clone())?;
                    self.literal_property(h, k.encode_utf16().collect(), v)?;
                }
                Ok(Value::Object(h))
            }
            E::Array(xs) => {
                let h = self.alloc(ObjectKind::Array(0), Some(self.array_proto))?;
                for (i, x) in xs.iter().enumerate() {
                    let v = self.expression(x, e, this.clone())?;
                    self.literal_property(h, i.to_string().encode_utf16().collect(), v)?;
                }
                Ok(Value::Object(h))
            }
        })();
        r.map_err(|err| self.locate(err, x.pos))
    }
    fn literal_property(&mut self, h: Handle, k: Key, v: Value) -> JsResult<()> {
        if !self.objects[h.1].props.contains_key(&k) {
            self.capacity()?;
            self.entries += 1;
        }
        let o = &mut self.objects[h.1];
        if let ObjectKind::Array(len) = &mut o.kind {
            if let Some(i) = Self::index(&k) {
                *len = (*len).max(i + 1);
            }
        }
        if let ObjectKind::Array(len) = o.kind {
            o.props.insert(
                "length".encode_utf16().collect(),
                Property::Data(Value::Number(len as f64), true, false, false),
            );
        }
        o.props.insert(k, Property::Data(v, true, true, true));
        Ok(())
    }
    fn member(x: &Expr) -> bool {
        match &x.kind {
            E::Member(_, _) => true,
            E::Group(x) => Self::member(x),
            _ => false,
        }
    }
    fn callable(&self, v: &Value) -> JsResult<ObjectKind> {
        if let Value::Object(h) = v {
            let kind = self.object(*h)?.kind.clone();
            if matches!(kind, ObjectKind::Function(_) | ObjectKind::Native(..)) {
                return Ok(kind);
            }
        }
        Err(Error::new(ErrorKind::Type, "value is not callable"))
    }
    pub fn call(&mut self, function: Value, this: Value, args: &[Value]) -> JsResult<Value> {
        self.frames.push(Frame::default());
        self.keep_ref(&function);
        self.keep_ref(&this);
        for v in args {
            self.keep_ref(v);
        }
        let r = self.call_inner(function, this, args);
        self.frames.pop();
        self.keep_result(&r);
        r
    }
    fn call_inner(&mut self, function: Value, this: Value, args: &[Value]) -> JsResult<Value> {
        self.check()?;
        if self.depth >= self.limits.call_depth {
            return Err(Error::new(ErrorKind::Limit, "call depth limit"));
        }
        self.validate(&this)?;
        for v in args {
            self.validate(v)?;
        }
        let kind = self.callable(&function)?;
        self.depth += 1;
        let r = (|| {
            self.charge()?;
            match kind {
                ObjectKind::Native(f, _) => f(self, this, args),
                ObjectKind::Function(f) => {
                    let old = self.source.clone();
                    self.source = f.source.clone();
                    let r = (|| {
                        let env = self.env(f.env)?;
                        for (i, n) in f.params.iter().enumerate() {
                            self.capacity()?;
                            self.entries += 1;
                            self.envs[env].bindings.insert(
                                n.clone(),
                                Binding {
                                    constant: false,
                                    value: Some(args.get(i).cloned().unwrap_or(Value::Undefined)),
                                },
                            );
                        }
                        let body_env = self.env(env)?;
                        self.prepare(body_env, &f.body)?;
                        match self.list(&f.body, body_env, f.lexical_this.unwrap_or(this))? {
                            Completion::Return(v) => Ok(v),
                            _ => Ok(Value::Undefined),
                        }
                    })();
                    let r = r.map_err(|e| self.locate(e, 0));
                    self.source = old;
                    r
                }
                _ => unreachable!(),
            }
        })();
        self.depth -= 1;
        let v = r?;
        self.validate(&v)?;
        Ok(v)
    }
}
