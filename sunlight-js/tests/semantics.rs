use std::{cell::RefCell, rc::Rc};
use sunlight_js::{Context, Error, ErrorKind, Limits, Value};
fn eval(s: &str) -> Value {
    Context::default().eval(s, "test.js").unwrap()
}
fn number(s: &str, n: f64) {
    assert_eq!(eval(s), Value::Number(n), "{s}");
}
fn yes(s: &str) {
    assert_eq!(eval(s), Value::Boolean(true), "{s}");
}
fn error(s: &str, k: ErrorKind) {
    assert_eq!(
        Context::default().eval(s, "bad.js").unwrap_err().kind,
        k,
        "{s}"
    );
}
#[test]
fn precedence() {
    number("2 + 3 * 4;", 14.0);
    number("(2 + 3) * 4;", 20.0);
    yes("2 + 2 === 4 && 3 < 4;");
    number("false ? 1 : true ? 2 : 3;", 2.0);
    number("let a; let b; a = b = 9; a + b;", 18.0);
}
#[test]
fn diagnostics() {
    let e = Context::default()
        .eval("let x = 1;\nlet y = ;", "location.js")
        .unwrap_err();
    assert_eq!(
        (&*e.source, e.line, e.column, e.offset),
        ("location.js", 2, 9, 19)
    );
    error("const x;", ErrorKind::Syntax);
    error("let x; let x;", ErrorKind::Syntax);
    error("break;", ErrorKind::Syntax);
    error("return 1;", ErrorKind::Syntax);
    error("for (;;) { function f() { break; } }", ErrorKind::Syntax);
    error("function f(x,x) {}", ErrorKind::Syntax);
    error("function f(x) { let x; }", ErrorKind::Syntax);
}
#[test]
fn lexical_scope() {
    number("let x = 1; { let x = 2; x = 3; } x;", 1.0);
    error("let x = x;", ErrorKind::Reference);
    error("let x = 1; { x; let x = 2; }", ErrorKind::Reference);
    error("{ x = 2; let x; }", ErrorKind::Reference);
    error("const x = 1; x = 2;", ErrorKind::Type);
    number("const o = {x: 1}; o.x = 2; o.x;", 2.0);
    number("{ let undefined = 3; undefined; }", 3.0); // separate inner lexical scope checked below
}
#[test]
fn functions() {
    number(
        "f(5); function f(n) { if (n < 2) { return 1; } return n * f(n - 1); }",
        120.0,
    );
    number(
        "const f = function self(n) { return n < 2 ? 1 : n * self(n-1); }; f(5);",
        120.0,
    );
    number("function make() { let n = 0; return () => { n = n + 1; return n; }; } const c = make(); c(); c();",2.0);
    number(
        "function f() { let x=1; { function g() { return x; } return g(); } } f();",
        1.0,
    );
    yes("function f() {} f() === undefined;");
}
#[test]
fn loops() {
    number(
        "let s=0; for (let i=0; i<6; i=i+1) { if(i===2) {continue;} if(i===5) {break;} s=s+i; } s;",
        8.0,
    );
    number("let x=0; while(x<3) { x=x+1; } x;", 3.0);
    number(
        "const fs=[]; for(let i=0;i<3;i=i+1) { fs[i]=()=>i; } fs[0]()*100+fs[1]()*10+fs[2]();",
        12.0,
    );
    number(
        "let init; function seed(f){init=f;return 0;} for(let i=seed(()=>i);i<1;i=i+1) {} init();",
        0.0,
    );
}
#[test]
fn values_and_short_circuit() {
    yes("null !== undefined && NaN !== NaN && 0 === -0;");
    number(
        "let n=0; false && (n=1); true || (n=2); 3 ?? (n=3); n;",
        0.0,
    );
    number("0 || 7;", 7.0);
    number("0 ?? 7;", 0.0);
    number("null ?? 7;", 7.0);
    assert_eq!(eval("'a'+2;"), Value::string("a2"));
    number("null + true + +'2';", 3.0);
    yes("'20' < '3';");
    yes("1/0 === Infinity;");
}
#[test]
fn this_rules() {
    yes("function f(){return this;} f() === undefined;");
    number("const o={x:7,m(){return this.x;}}; (o.m)();", 7.0);
    yes("const o={m(){return this;}}; const f=o.m; f()===undefined;");
    number(
        "const o={x:9,m(){return ()=>this.x;}};const f=o.m();f();",
        9.0,
    );
    yes("const o={f:()=>this};o.f()===globalThis;");
}
#[test]
fn evaluation_order() {
    number("let n=0;const a={};function base(){n=n*10+1;return a;}function key(){n=n*10+2;return 'x';}function rhs(){n=n*10+3;return 8;}base()[key()]=rhs();n;",123.0);
    number(
        "let n=0;function arg(x){n=n*10+x;return x;}function f(a,b){return n;}f(arg(1),arg(2));",
        12.0,
    );
    number("let o={x:1};let old=o;o.x=(o={x:2});old.x.x;", 2.0);
}
#[test]
fn objects_arrays_strings() {
    yes("const a={};const b={};a===a && a!==b;");
    yes("const a={x:undefined};a.x===undefined && a.missing===undefined;");
    number("const a=[1,2];a[4]=9;a.length;", 5.0);
    yes("const a=[1,2,3];a.length=1;a[2]===undefined;");
    error("const a=[];a.length=1.5;", ErrorKind::Range);
    number("'😀'.length;", 2.0);
    yes("'😀'[0] === '\\uD83D' && '😀'[1] === '\\uDE00';");
    yes("const o={};o['\\uD800']=3;o['\\uD800']===3;");
    error("null.x;", ErrorKind::Type);
    error("'a'.x=2;", ErrorKind::Type);
}
#[test]
fn host_and_prototypes() {
    let mut c = Context::default();
    let proto = c.create_object(Some(c.object_prototype())).unwrap();
    c.define_data(proto, "x", Value::Number(7.0), true).unwrap();
    let o = c.create_object(Some(proto)).unwrap();
    c.set_global("host", Value::Object(o)).unwrap();
    assert_eq!(c.eval("host.x;", "host.js").unwrap(), Value::Number(7.0));
    c.eval("host.x=9;", "host.js").unwrap();
    assert_eq!(
        c.get_property(Value::Object(proto), "x").unwrap(),
        Value::Number(7.0)
    );
    c.define_data(o, "empty", Value::Undefined, true).unwrap();
    assert!(c.has_own(o, "empty").unwrap());
    assert!(!c.has_own(o, "missing").unwrap());
    let state = Rc::new(RefCell::new(Value::Number(1.0)));
    let read = state.clone();
    let write = state.clone();
    let get = c
        .native_function(move |_, _, _| Ok(read.borrow().clone()))
        .unwrap();
    let set = c
        .native_function(move |_, _, args| {
            *write.borrow_mut() = args[0].clone();
            Ok(Value::Undefined)
        })
        .unwrap();
    c.define_accessor(o, "state", Some(get), Some(set)).unwrap();
    assert_eq!(
        c.eval("host.state=42;host.state;", "host.js").unwrap(),
        Value::Number(42.0)
    );
    c.register_native("fail", |_, _, _| {
        Err(Error::new(ErrorKind::Type, "native failure"))
    })
    .unwrap();
    let e = c.eval("fail();", "native.js").unwrap_err();
    assert_eq!(e.kind, ErrorKind::Type);
    assert_eq!(e.source, "native.js");
    c.register_native("nested", |c, _, _| c.eval("host.state;", "nested.js"))
        .unwrap();
    assert_eq!(
        c.eval("nested();", "outer.js").unwrap(),
        Value::Number(42.0)
    );
    let foreign = Context::default().create_object(None).unwrap();
    assert!(c.set_global("foreign", Value::Object(foreign)).is_err());
}
#[test]
fn exceptions_and_limits() {
    let e = Context::default()
        .eval("function f(){throw 7;}f();", "throw.js")
        .unwrap_err();
    assert_eq!(e.thrown, Some(Value::Number(7.0)));
    for s in [
        "while(true) {}",
        "for(;;) {}",
        "function f(){return f();} f();",
    ] {
        error(s, ErrorKind::Limit);
    }
    let mut c = Context::new(Limits {
        arena_entries: 20,
        ..Limits::default()
    });
    assert_eq!(
        c.eval("while(true) {let o={};}", "arena.js")
            .unwrap_err()
            .kind,
        ErrorKind::Limit
    );
    assert!(Context::default()
        .eval(
            &format!("{}0{}", "(".repeat(200), ")".repeat(200)),
            "deep.js"
        )
        .is_err());
}
#[test]
fn teardown() {
    struct Probe(Rc<RefCell<usize>>);
    impl Drop for Probe {
        fn drop(&mut self) {
            *self.0.borrow_mut() += 1;
        }
    }
    let drops = Rc::new(RefCell::new(0));
    for _ in 0..200 {
        let mut c = Context::default();
        let p = Probe(drops.clone());
        c.register_native("probe", move |_, _, _| {
            let _ = &p;
            Ok(Value::Undefined)
        })
        .unwrap();
        c.eval(
            "let o={};o.self=o;function f(){return o;}o.f=f;",
            "cycle.js",
        )
        .unwrap();
        c.close().unwrap();
        assert!(c.eval("1;", "closed.js").is_err());
        assert_eq!(c.arena_usage(), (0, 0));
    }
    assert_eq!(*drops.borrow(), 200);
}
#[test]
fn unsupported_is_rejected_before_side_effects() {
    for s in [
        "var x=1;",
        "with({}) {}",
        "010;",
        "'\\1';",
        "class X {}",
        "import 'file';",
        "export {};",
        "let {x}=o;",
        "[...a];",
        "a?.b;",
        "`template`;",
        "async function f(){}",
        "function* f(){}",
        "try {} catch(e) {}",
        "new Object();",
        "a == b;",
        "a++;",
        "let x=1; x+=1;",
        "if(true) function f(){}",
        "true ?? false || true;",
    ] {
        let mut c = Context::default();
        let seen = Rc::new(RefCell::new(false));
        let flag = seen.clone();
        c.register_native("mark", move |_, _, _| {
            *flag.borrow_mut() = true;
            Ok(Value::Undefined)
        })
        .unwrap();
        assert!(
            c.eval(&format!("mark(); {s}"), "unsupported.js").is_err(),
            "{s}"
        );
        assert!(!*seen.borrow(), "{s}");
    }
}

#[test]
fn callback_roots_and_reentry_guards() {
    let mut c = Context::default();
    c.register_native("bounce", |c, _, args| {
        assert!(c.close().is_err());
        c.eval("const garbage = [{}, {}, {}];", "inside.js")?;
        c.get_property(args[0].clone(), "value")
    })
    .unwrap();
    assert_eq!(
        c.eval("bounce({value: 23});", "roots.js").unwrap(),
        Value::Number(23.0)
    );
    c.register_native("again", |c, _, _| c.eval("again();", "reentry.js"))
        .unwrap();
    assert_eq!(
        c.eval("again();", "outer.js").unwrap_err().kind,
        ErrorKind::Limit
    );
    assert_eq!(
        c.eval("6*7;", "after-error.js").unwrap(),
        Value::Number(42.0)
    );
}

#[test]
fn property_descriptors_and_order() {
    let mut c = Context::default();
    let proto = c.create_object(Some(c.object_prototype())).unwrap();
    c.define_data(proto, "locked", Value::Number(3.0), false)
        .unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let out = seen.clone();
    let setter = c
        .native_function(move |_, receiver, args| {
            out.borrow_mut().push((receiver, args[0].clone()));
            Ok(Value::Undefined)
        })
        .unwrap();
    c.define_accessor(proto, "field", None, Some(setter))
        .unwrap();
    let child = c.create_object(Some(proto)).unwrap();
    c.set_global("child", Value::Object(child)).unwrap();
    c.eval("child.field=8;", "setter.js").unwrap();
    assert_eq!(seen.borrow()[0], (Value::Object(child), Value::Number(8.0)));
    assert_eq!(
        c.eval("child.locked=4;", "readonly.js").unwrap_err().kind,
        ErrorKind::Type
    );
    assert!(c.define_data(proto, "locked", Value::Null, true).is_err());
    c.register_native("rhs", |_, _, _| {
        Err(Error::new(ErrorKind::Exception, "RHS evaluated"))
    })
    .unwrap();
    assert_eq!(
        c.eval("missing=rhs();", "order.js").unwrap_err().kind,
        ErrorKind::Exception
    );
}

#[test]
fn numeric_strings_and_source_limits() {
    for source in ["1e21", "1e-7", "1e-6", "-1/0", "1e30"] {
        assert!(!eval(source).display().contains("inf"));
    }
    let mut c = Context::new(Limits {
        source_bytes: 3,
        ..Limits::default()
    });
    assert_eq!(
        c.eval("1234", "large.js").unwrap_err().kind,
        ErrorKind::Limit
    );
    let mut c = Context::new(Limits {
        string_units: 4,
        ..Limits::default()
    });
    assert_eq!(
        c.eval("'abc'+'def';", "large-string.js").unwrap_err().kind,
        ErrorKind::Limit
    );
    let deep = (0..150).map(|_| "1+").collect::<String>() + "1";
    assert_eq!(
        Context::default().eval(&deep, "chain.js").unwrap_err().kind,
        ErrorKind::Limit
    );
    assert_eq!(
        eval("function f(){return\r9;} f()===undefined;"),
        Value::Boolean(true)
    );
    assert_eq!(
        eval("function f(){return\u{2028}9;} f()===undefined;"),
        Value::Boolean(true)
    );
}

#[test]
fn deferred_property_key_conversion() {
    assert_eq!(eval("let s='';const key={toString(){s=s+'k';return 'x';}};const o={};function rhs(){s=s+'r';return 1;}o[key]=rhs();s"),Value::string("rk"));
    let mut c = Context::default();
    c.register_native("rhs", |_, _, _| {
        Err(Error::new(ErrorKind::Exception, "RHS evaluated"))
    })
    .unwrap();
    assert_eq!(
        c.eval("null.x=rhs();", "null-write.js").unwrap_err().kind,
        ErrorKind::Exception
    );
}
