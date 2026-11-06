use std::{cell::Cell, rc::Rc};
use sunlight_js::{Context, ErrorKind, Limits, Value};

#[test]
fn object_and_closure_cycles_return_to_baseline() {
    let mut js = Context::default();
    let baseline = js.collect_garbage().unwrap();
    for _ in 0..200 {
        js.eval(
            "{let a={};let b=[];a.b=b;b[0]=a;let f=function self(){return ()=>f;};a.f=f;}",
            "cycles",
        )
        .unwrap();
        let stats = js.collect_garbage().unwrap();
        assert!(stats.reclaimed_objects >= 4);
        assert!(stats.reclaimed_environments >= 2);
        assert_eq!(stats.live_objects, baseline.live_objects);
        assert_eq!(stats.live_environments, baseline.live_environments);
        assert_eq!(stats.live_entries, baseline.live_entries);
        assert!(stats.object_slots <= baseline.object_slots + 8);
        assert!(stats.environment_slots <= baseline.environment_slots + 3);
    }
}

#[test]
fn pins_stale_handles_and_destructors() {
    let mut js = Context::default();
    let captured = Rc::new(Cell::new(0));
    struct DropFlag(Rc<Cell<usize>>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let flag = DropFlag(captured.clone());
    let callback = js
        .native_function(move |_, _, _| {
            let _ = &flag;
            Ok(Value::Undefined)
        })
        .unwrap();
    let h = js.create_object(None).unwrap();
    js.define_data(h, "self", Value::Object(h), true).unwrap();
    js.define_data(h, "callback", callback, true).unwrap();
    let pin = js.register_root(Value::Object(h)).unwrap();
    let clone = pin.clone();
    drop(pin);
    for _ in 0..3 {
        js.collect_garbage().unwrap();
        assert!(js.has_own(h, "self").unwrap());
    }
    drop(clone);
    assert_eq!(js.collect_garbage().unwrap().reclaimed_objects, 2);
    assert_eq!(captured.get(), 1);
    let replacement = js.create_object(None).unwrap();
    assert_ne!(replacement, h);
    assert_eq!(
        js.get_property(Value::Object(h), "self").unwrap_err().kind,
        ErrorKind::Reference
    );
    let mut other = Context::default();
    assert!(other.register_root(Value::Object(replacement)).is_err());
    js.close().unwrap();
    assert!(js.register_root(Value::Object(replacement)).is_err());
}

#[test]
fn every_safe_point_preserves_temporaries_accessors_frames_and_reentry() {
    let mut js = Context::default();
    js.set_gc_threshold(0);
    js.register_native("gc", |js, _, _| {
        js.collect_garbage()?;
        Ok(Value::Undefined)
    })
    .unwrap();
    js.register_native("reenter", |js, _, _| js.eval("gc();({v:8})", "nested"))
        .unwrap();
    let result = js
        .eval(
            r#"
        function f(a,b){gc();return a.x+b.v;}
        const fs=[];for(let i=0;i<4;i=i+1){fs.push(()=>{gc();return i;});}
        const o={x:3};
        Object.defineProperty(o,'y',{get:function(){gc();return this.x;}});
        f({x:o.y},reenter())===11 && fs[0]()===0 && fs[3]()===3 &&
        ({a:{x:4},b:reenter()}).a.x===4 &&
        [({x:5}),reenter()][0].x===5
    "#,
            "stress",
        )
        .unwrap();
    assert_eq!(result, Value::Boolean(true));
    let thrown = js
        .eval("throw {payload:42}", "throw")
        .unwrap_err()
        .thrown
        .unwrap();
    let root = js.register_root(thrown).unwrap();
    js.collect_garbage().unwrap();
    assert_eq!(
        js.get_property(root.value(), "payload").unwrap(),
        Value::Number(42.0)
    );
}

#[test]
fn automatic_collection_bounds_loop_heap_and_reuses_environment_slots() {
    let mut js = Context::new(Limits {
        fuel: 1_000_000,
        arena_entries: 1500,
        ..Limits::default()
    });
    js.set_gc_threshold(32);
    let result = js
        .eval(
            "let n=0;for(let i=0;i<2000;i=i+1){let a={};let b={};a.b=b;b.a=a;n=n+1;}n",
            "growth",
        )
        .unwrap();
    assert_eq!(result, Value::Number(2000.0));
    let stats = js.collect_garbage().unwrap();
    assert!(stats.collections > 50);
    assert!(stats.object_slots < 100);
    assert!(stats.environment_slots < 100);
}

#[test]
fn iterative_trace_and_native_capture_edges() {
    let mut js = Context::default();
    let baseline = js.collect_garbage().unwrap().live_objects;
    let first = js.create_object(None).unwrap();
    let mut previous = first;
    for _ in 0..20_000 {
        let next = js.create_object(None).unwrap();
        js.define_data(previous, "next", Value::Object(next), false)
            .unwrap();
        previous = next;
    }
    js.define_data(previous, "next", Value::Object(first), false)
        .unwrap();
    let native = js
        .native_function_with_roots(vec![Value::Object(first)], move |js, _, _| {
            js.get_property(Value::Object(first), "next")
        })
        .unwrap();
    let root = js.register_root(native).unwrap();
    assert_eq!(
        js.collect_garbage().unwrap().live_objects,
        baseline + 20_002
    );
    js.call(root.value(), Value::Undefined, &[]).unwrap();
    drop(root);
    assert_eq!(js.collect_garbage().unwrap().live_objects, baseline);
}

#[test]
fn bounded_host_turns_share_fuel_with_reentrant_evaluation() {
    let mut js = Context::default();
    js.register_native("nested", |js, _, _| js.eval("while(true){}", "nested"))
        .unwrap();
    let error = js.host_turn(|js| js.eval("nested()", "outer")).unwrap_err();
    assert_eq!(error.kind, sunlight_js::ErrorKind::Limit);
    assert_eq!(
        js.host_turn(|js| js.eval("40+2", "fresh")).unwrap(),
        Value::Number(42.0)
    );
}
