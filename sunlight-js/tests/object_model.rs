use std::{cell::RefCell, rc::Rc};
use sunlight_js::{Context, ErrorKind, PropertyDescriptor, Value};

fn yes(source: &str) {
    assert_eq!(
        Context::default().eval(source, "objects.js").unwrap(),
        Value::Boolean(true),
        "{source}"
    );
}
fn fails(source: &str, kind: ErrorKind) {
    assert_eq!(
        Context::default()
            .eval(source, "objects.js")
            .unwrap_err()
            .kind,
        kind,
        "{source}"
    );
}

#[test]
fn intrinsic_hierarchy_and_shadowing() {
    yes("Object.getPrototypeOf({})===Object.prototype && Object.getPrototypeOf(Object.prototype)===null");
    yes("Object.getPrototypeOf([])===Array.prototype && Object.getPrototypeOf(Array.prototype)===Object.prototype");
    yes("function f(){};Object.getPrototypeOf(f)===Function.prototype && Object.getPrototypeOf(Function.prototype)===Object.prototype && Function.prototype()===undefined");
    yes("Object.getPrototypeOf(Object)===Function.prototype && Object.getPrototypeOf(Array)===Function.prototype && Object.getPrototypeOf(Function)===Function.prototype");
    yes("const node={x:1};const element=Object.create(node);const html=Object.create(element);html.x=2;html.x===2 && node.x===1 && html.hasOwnProperty('x') && !element.hasOwnProperty('x')");
    yes("const p={x:undefined};const o=Object.create(p);!o.hasOwnProperty('x') && p.hasOwnProperty('x') && Object.getPrototypeOf(o)===p");
    yes("const o=Object.create(null);o.toString===undefined && Object.getPrototypeOf(o)===null");
    yes("({}).toString()==='[object Object]' && Object.prototype.toString.call([])==='[object Array]' && Object.prototype.toString.call(null)==='[object Null]'");
    yes("const d=Object.getOwnPropertyDescriptor(Array.prototype,'push');d.writable && !d.enumerable && d.configurable");
}

#[test]
fn js_accessors_and_receiver() {
    yes("const p={};Object.defineProperty(p,'x',{get:function(){return this.value;},set:function(v){this.value=v;}});const o=Object.create(p);o.x=42;o.x===42 && o.value===42 && p.value===undefined && !o.hasOwnProperty('x')");
    yes("const o={};let seen;Object.defineProperty(o,'x',{get:function(){seen=this;return 7;}});o.x===7 && seen===o");
    yes("const p={};Object.defineProperty(p,'x',{set:function(v){this.y=v;}});const o=Object.create(p);o.x=4;o.x===undefined && o.y===4");
    fails(
        "const p={};Object.defineProperty(p,'x',{get:function(){return 1;}});Object.create(p).x=2",
        ErrorKind::Type,
    );
    fails("Object.defineProperty({},'x',{get:1})", ErrorKind::Type);
    fails(
        "Object.defineProperty({},'x',{get:undefined,value:1})",
        ErrorKind::Type,
    );
    yes("const p={value:7,writable:true};const d=Object.create(p);const o={};Object.defineProperty(o,'x',d);o.x=9;o.x===9");
    yes("let s='';const d={};Object.defineProperty(d,'enumerable',{get:function(){s=s+'e';return true;}});Object.defineProperty(d,'value',{get:function(){s=s+'v';return 3;}});const o={};Object.defineProperty(o,'x',d);s==='ev' && o.x===3");
}

#[test]
fn descriptor_defaults_redefinition_and_same_value() {
    yes("const o={};Object.defineProperty(o,'x',{value:3});const d=Object.getOwnPropertyDescriptor(o,'x');d.value===3 && !d.writable && !d.enumerable && !d.configurable");
    fails(
        "const o={};Object.defineProperty(o,'x',{value:3});o.x=4",
        ErrorKind::Type,
    );
    fails(
        "const o={};Object.defineProperty(o,'x',{value:3});Object.defineProperty(o,'x',{value:4})",
        ErrorKind::Type,
    );
    fails(
        "const o={};Object.defineProperty(o,'x',{});Object.defineProperty(o,'x',{enumerable:true})",
        ErrorKind::Type,
    );
    fails(
        "const o={};Object.defineProperty(o,'x',{value:0});Object.defineProperty(o,'x',{value:-0})",
        ErrorKind::Type,
    );
    yes("const o={};Object.defineProperty(o,'x',{value:NaN});Object.defineProperty(o,'x',{value:NaN})===o");
    yes("const o={x:1};Object.defineProperty(o,'x',{enumerable:false});o.x=2;const d=Object.getOwnPropertyDescriptor(o,'x');d.value===2 && !d.enumerable && d.writable && d.configurable");
    yes("const o={};Object.defineProperty(o,'x',{value:1,writable:true});Object.defineProperty(o,'x',{value:2,writable:false});o.x===2");
    yes("const o={x:1};Object.defineProperty(o,'x',{get:function(){return 4;}});o.x===4 && Object.getOwnPropertyDescriptor(o,'x').value===undefined");
    yes("const o={};Object.defineProperty(o,'x',{get:function(){return 1;},configurable:true});Object.defineProperty(o,'x',{value:5});o.x===5 && !Object.getOwnPropertyDescriptor(o,'x').writable");
    fails(
        "const p={};Object.defineProperty(p,'x',{value:1});Object.create(p).x=2",
        ErrorKind::Type,
    );
}

#[test]
fn array_methods_sparse_indices_and_length_descriptors() {
    yes("const a=[1,2];let n=0;const v={valueOf(){n=n+1;return n===1?4294967296:0;}};Object.defineProperty(a,'length',{value:v});a.length===0 && n===2");
    yes("const a=[1,2];const v={valueOf(){Object.defineProperty(a,'length',{writable:false});return 2;}};Object.defineProperty(a,'length',{value:v});a.length===2 && !Object.getOwnPropertyDescriptor(a,'length').writable");
    yes("const a=[1];a.push(2,3)===3 && a.pop()===3 && a.length===2 && a[1]===2");
    yes("const a=[];a.pop()===undefined && a.length===0 && a.push()===0");
    yes("const a=[1,2,3,4];const b=a.slice(1,-1);b.length===2 && b[0]===2 && b[1]===3 && a.length===4");
    yes("const a=[];a.length=4;a[2]=8;const b=a.slice(1,4);b.length===3 && !b.hasOwnProperty('0') && b[1]===8 && !b.hasOwnProperty('2')");
    yes("const a=[1,2];Object.defineProperty(a,'4',{value:7,configurable:true});a.length===5 && a[4]===7");
    yes("const a=[1,2,3];Object.defineProperty(a,'length',{value:1});const d=Object.getOwnPropertyDescriptor(a,'length');a[1]===undefined && d.value===1 && d.writable && !d.enumerable && !d.configurable");
    fails(
        "const a=[];Object.defineProperty(a,'length',{writable:false});a.push(1)",
        ErrorKind::Type,
    );
    fails(
        "const a=[1];Object.defineProperty(a,'length',{writable:false});a.length=1",
        ErrorKind::Type,
    );
    fails(
        "const a=[];Object.defineProperty(a,'length',{value:1.5})",
        ErrorKind::Range,
    );
    fails(
        "const a=[];Object.defineProperty(a,'length',{get:function(){return 1;}})",
        ErrorKind::Type,
    );
    let mut c = Context::default();
    c.eval(
        "const a=[0,1,2,3];Object.defineProperty(a,'1',{configurable:false});",
        "array.js",
    )
    .unwrap();
    assert_eq!(
        c.eval(
            "Object.defineProperty(a,'length',{value:0,writable:false});",
            "array.js"
        )
        .unwrap_err()
        .kind,
        ErrorKind::Type
    );
    assert_eq!(c.eval("a.length===2 && a[1]===1 && !a.hasOwnProperty('2') && !Object.getOwnPropertyDescriptor(a,'length').writable", "array.js").unwrap(), Value::Boolean(true));
    yes("const a={length:0};Array.prototype.push.call(a,5,6);Array.prototype.pop.call(a)===6 && a.length===1 && !a.hasOwnProperty('1')");
}

#[test]
fn call_apply_and_strict_receivers() {
    yes("function f(a,b){return this.x+a+b;}const o={x:3};f.call(o,4,5)===12 && f.apply(o,[4,5])===12");
    yes("function f(){return this;}f.call(null)===null && f.apply(undefined,null)===undefined");
    yes("function f(a,b){return a===undefined && b===undefined;}f.apply(null,[]) && f.call(null)");
    yes("function f(a,b){return a+b;}f.apply(null,{0:2,1:3,length:2})===5");
    yes("const f=()=>this;f.call(null)===globalThis");
    fails("Function.prototype.call.call({},null)", ErrorKind::Type);
    fails("function f(){};f.apply(null,2)", ErrorKind::Type);
}

#[test]
fn native_accessors_on_wrapper_chain_and_validation() {
    let mut c = Context::default();
    let node = c.create_object(Some(c.object_prototype())).unwrap();
    let element = c.create_object(Some(node)).unwrap();
    let html = c.create_object(Some(element)).unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let reads = seen.clone();
    let get = c
        .native_function(move |c, receiver, _| {
            reads.borrow_mut().push(receiver.clone());
            c.get_property(receiver, "storage")
        })
        .unwrap();
    let writes = seen.clone();
    let set = c
        .native_function(move |c, receiver, args| {
            writes.borrow_mut().push(receiver.clone());
            c.set_property(receiver, "storage", args[0].clone())?;
            Ok(Value::Undefined)
        })
        .unwrap();
    c.define_property(
        node,
        "textContent",
        PropertyDescriptor {
            get: Some(get),
            set: Some(set),
            enumerable: Some(true),
            configurable: Some(true),
            ..PropertyDescriptor::default()
        },
    )
    .unwrap();
    c.set_global("wrapper", Value::Object(html)).unwrap();
    assert_eq!(
        c.eval("wrapper.textContent='hello';wrapper.textContent", "host.js")
            .unwrap(),
        Value::string("hello")
    );
    assert_eq!(
        *seen.borrow(),
        vec![Value::Object(html), Value::Object(html)]
    );
    assert!(!c.has_own(html, "textContent").unwrap());
    let mut other = Context::default();
    let foreign = other
        .native_function(|_, _, _| Ok(Value::Undefined))
        .unwrap();
    assert_eq!(
        c.define_accessor(node, "bad", Some(foreign), None)
            .unwrap_err()
            .kind,
        ErrorKind::Reference
    );
    c.close().unwrap();
    assert!(c.get_property(Value::Object(html), "textContent").is_err());
}
