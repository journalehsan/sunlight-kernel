//! Safe, non-moving arena and iterative mark/sweep. No Rust pointer is a JS edge.
use super::*;
use std::ops::{Index, IndexMut};

#[derive(Clone, Copy, Debug)]
pub(super) enum CellType {
    Object,
    Environment,
}
struct Header {
    marked: bool,
    next_free: Option<usize>,
    generation: u64,
    cell_type: CellType,
}
struct Cell<T> {
    header: Header,
    value: Option<T>,
}
pub(super) struct Arena<T> {
    cells: Vec<Cell<T>>,
    free: Option<usize>,
    kind: CellType,
    live: usize,
}
impl<T> Arena<T> {
    pub fn new(kind: CellType) -> Self {
        Self {
            cells: Vec::new(),
            free: None,
            kind,
            live: 0,
        }
    }
    pub fn singleton(kind: CellType, value: T) -> Self {
        let mut arena = Self::new(kind);
        arena.push(value);
        arena
    }
    pub fn push(&mut self, value: T) -> usize {
        self.live += 1;
        if let Some(index) = self.free {
            let cell = &mut self.cells[index];
            self.free = cell.header.next_free.take();
            cell.value = Some(value);
            index
        } else {
            let index = self.cells.len();
            self.cells.push(Cell {
                header: Header {
                    marked: false,
                    next_free: None,
                    generation: 0,
                    cell_type: self.kind,
                },
                value: Some(value),
            });
            index
        }
    }
    pub fn live(&self) -> usize {
        self.live
    }
    pub fn generation(&self, index: usize) -> u64 {
        self.cells[index].header.generation
    }
    pub fn get_handle(&self, index: usize, generation: u64) -> Option<&T> {
        let cell = self.cells.get(index)?;
        (cell.header.generation == generation)
            .then_some(cell.value.as_ref())
            .flatten()
    }
    fn mark(&mut self, index: usize) -> bool {
        let Some(cell) = self.cells.get_mut(index) else {
            return false;
        };
        if cell.value.is_none() || cell.header.marked {
            return false;
        }
        debug_assert_eq!(
            std::mem::discriminant(&cell.header.cell_type),
            std::mem::discriminant(&self.kind)
        );
        cell.header.marked = true;
        true
    }
    fn sweep(&mut self) -> usize {
        let mut reclaimed = 0;
        for (index, cell) in self.cells.iter_mut().enumerate() {
            if cell.value.is_some() && !cell.header.marked {
                // Dropping the owned payload runs all Rust destructors. Graph
                // edges are integers, so dropping a deep JS graph cannot recurse.
                cell.value.take();
                self.live -= 1;
                reclaimed += 1;
                if let Some(generation) = cell.header.generation.checked_add(1) {
                    cell.header.generation = generation;
                    cell.header.next_free = self.free;
                    self.free = Some(index);
                }
            }
            cell.header.marked = false;
        }
        reclaimed
    }
    pub fn clear(&mut self) {
        self.cells.clear();
        self.free = None;
        self.live = 0;
    }
}
impl<T> Index<usize> for Arena<T> {
    type Output = T;
    fn index(&self, index: usize) -> &T {
        self.cells[index]
            .value
            .as_ref()
            .expect("live internal GC edge")
    }
}
impl<T> IndexMut<usize> for Arena<T> {
    fn index_mut(&mut self, index: usize) -> &mut T {
        self.cells[index]
            .value
            .as_mut()
            .expect("live internal GC edge")
    }
}

/// A host pin. Clones share one registration; dropping the last clone releases it.
/// It owns no Context and becomes invalid when the Context closes.
#[derive(Clone, Debug)]
pub struct HostRoot(Rc<Value>);
impl HostRoot {
    pub fn value(&self) -> Value {
        (*self.0).clone()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GcStats {
    pub reclaimed_objects: usize,
    pub reclaimed_environments: usize,
    pub live_objects: usize,
    pub live_environments: usize,
    pub live_entries: usize,
    pub object_slots: usize,
    pub environment_slots: usize,
    pub collections: usize,
}
#[derive(Default)]
pub(super) struct Frame {
    values: Vec<Handle>,
    envs: Vec<usize>,
}
enum Edge {
    Object(Handle),
    Env(usize),
}
impl Context {
    pub fn register_root(&mut self, value: Value) -> JsResult<HostRoot> {
        self.check()?;
        self.validate(&value)?;
        let root = HostRoot(Rc::new(value));
        self.roots.retain(|root| root.strong_count() != 0);
        self.roots.push(Rc::downgrade(&root.0));
        Ok(root)
    }
    /// Collection runs at interpreter safe points after this many cell allocations.
    /// Zero requests collection at every fuel charge (useful for stress tests).
    pub fn set_gc_threshold(&mut self, allocations: usize) {
        self.gc_threshold = allocations;
    }
    pub(super) fn keep_ref(&mut self, value: &Value) {
        if let Value::Object(h) = value {
            self.keep(Value::Object(*h));
        }
    }
    pub(super) fn keep(&mut self, value: Value) {
        if let (Some(frame), Value::Object(handle)) = (self.frames.last_mut(), value) {
            frame.values.push(handle);
        }
    }
    pub(super) fn keep_env(&mut self, env: usize) {
        if let Some(frame) = self.frames.last_mut() {
            frame.envs.push(env);
        }
    }
    pub(super) fn keep_result(&mut self, result: &JsResult<Value>) {
        match result {
            Ok(v) => self.keep_ref(v),
            Err(e) => {
                if let Some(v) = &e.thrown {
                    self.keep_ref(v);
                }
            }
        }
    }
    pub(super) fn keep_completion(&mut self, result: &JsResult<Completion>) {
        let value = match result {
            Ok(Completion::Normal(v) | Completion::Break(v) | Completion::Continue(v)) => {
                v.as_ref()
            }
            Ok(Completion::Return(v)) => Some(v),
            Err(e) => e.thrown.as_ref(),
        };
        if let Some(v) = value {
            self.keep_ref(v);
        }
    }
    pub(super) fn reset_frame(&mut self, env: usize, this: &Value, last: Option<&Value>) {
        if let Some(frame) = self.frames.last_mut() {
            frame.values.clear();
            frame.envs.clear();
        }
        self.keep_env(env);
        self.keep_ref(this);
        if let Some(v) = last {
            self.keep_ref(v);
        }
    }
    pub fn collect_garbage(&mut self) -> JsResult<GcStats> {
        self.check()?;
        let mut gray = vec![Edge::Env(0)];
        for h in [
            self.global,
            self.object_proto,
            self.array_proto,
            self.function_proto,
        ] {
            gray.push(Edge::Object(h));
        }
        for frame in &self.frames {
            gray.extend(frame.values.iter().copied().map(Edge::Object));
            gray.extend(frame.envs.iter().copied().map(Edge::Env));
        }
        self.roots.retain(|root| {
            if let Some(value) = root.upgrade() {
                if let Value::Object(h) = *value {
                    gray.push(Edge::Object(h));
                }
                true
            } else {
                false
            }
        });
        fn value(gray: &mut Vec<Edge>, v: &Value) {
            if let Value::Object(h) = v {
                gray.push(Edge::Object(*h));
            }
        }
        while let Some(edge) = gray.pop() {
            match edge {
                Edge::Object(h) => {
                    if h.0 != self.id
                        || self.objects.get_handle(h.1, h.2).is_none()
                        || !self.objects.mark(h.1)
                    {
                        continue;
                    }
                    let object = &self.objects[h.1];
                    if let Some(p) = object.prototype {
                        gray.push(Edge::Object(p));
                    }
                    if let ObjectKind::Function(f) = &object.kind {
                        gray.push(Edge::Env(f.env));
                        if let Some(v) = &f.lexical_this {
                            value(&mut gray, v);
                        }
                    }
                    if let ObjectKind::Native(_, captures) = &object.kind {
                        for v in captures {
                            value(&mut gray, v);
                        }
                    }
                    for p in object.props.values() {
                        match p {
                            Property::Data(v, ..) => value(&mut gray, v),
                            Property::Accessor(get, set, ..) => {
                                for v in get.iter().chain(set.iter()) {
                                    value(&mut gray, v);
                                }
                            }
                            Property::GlobalBinding(_) => gray.push(Edge::Env(0)),
                        }
                    }
                }
                Edge::Env(e) => {
                    if !self.envs.mark(e) {
                        continue;
                    }
                    let env = &self.envs[e];
                    if let Some(parent) = env.parent {
                        gray.push(Edge::Env(parent));
                    }
                    for b in env.bindings.values() {
                        if let Some(v) = &b.value {
                            value(&mut gray, v);
                        }
                    }
                }
            }
        }
        let reclaimed_objects = self.objects.sweep();
        let reclaimed_environments = self.envs.sweep();
        self.entries = self
            .objects
            .cells
            .iter()
            .filter_map(|c| c.value.as_ref())
            .map(|o| 1 + o.props.len())
            .sum::<usize>()
            + self
                .envs
                .cells
                .iter()
                .filter_map(|c| c.value.as_ref())
                .map(|e| 1 + e.bindings.len())
                .sum::<usize>();
        self.gc_allocations = 0;
        self.collections += 1;
        Ok(GcStats {
            reclaimed_objects,
            reclaimed_environments,
            live_objects: self.objects.live(),
            live_environments: self.envs.live(),
            live_entries: self.entries,
            object_slots: self.objects.cells.len(),
            environment_slots: self.envs.cells.len(),
            collections: self.collections,
        })
    }
}
