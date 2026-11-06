"""Selected deterministic subset comparisons with Node/V8; not Test262 conformance."""
import json
import pathlib
import subprocess

root = pathlib.Path(__file__).resolve().parents[1]
runner = root / "target/x86_64-unknown-linux-gnu/debug/sunjs"
cases = [
    "1e21", "1e-7", "1e-6", "-1/0", "1e30",
    "while(true){1;break;}", "while(true){1;if(true)break;}",
    "function f(){}; f === globalThis.f", "function f(){};globalThis.f=3;f",
    "+'0b101'", "+'0o17'", "+'0xFF'",
    "const o={m(){}};o.m.prototype===undefined",
    "let s='';const key={toString(){s=s+'k';return 'x';}};const o={};function rhs(){s=s+'r';return 1;}o[key]=rhs();s",
    "2+3*4", "(2+3)*4", "1/0", "0/0", "null===undefined", "0===-0",
    "let a; let b; a=b=9; a+b", "let x=1; {let x=2;} x",
    "let a=0; false && (a=1); true || (a=2); a", "0??7", "null??7",
    "'a'+2", "null+true+ +'2'", "'20'<'3'", "true?2:3",
    "function f(n){return n<2?1:n*f(n-1);} f(6)",
    "const f=function self(n){return n<2?1:n*self(n-1);};f(5)",
    "function make(){let n=0;return ()=>{n=n+1;return n;};}const f=make();f();f()",
    "const fs=[];for(let i=0;i<3;i=i+1){fs[i]=()=>i;}fs[0]()*100+fs[1]()*10+fs[2]()",
    "let n=0;while(n<3){n=n+1;}n",
    "let s=0;for(let i=0;i<6;i=i+1){if(i===2){continue;}if(i===5){break;}s=s+i;}s",
    "const o={x:7,m(){return this.x;}};(o.m)()",
    "function f(){return this;}f()===undefined",
    "const o={x:9,m(){return ()=>this.x;}};const f=o.m();f()",
    "const o={f:()=>this};o.f()===globalThis",
    "const a={};const b={};a===a&&a!==b",
    "const a=[1,2];a[4]=9;a.length", "const a=[1,2,3];a.length=1;a[2]===undefined",
    "'😀'.length", "'😀'[0]==='\\uD83D'", "const o={};o['\\uD800']=3;o['\\uD800']",
    "let n=0;const a={};function base(){n=n*10+1;return a;}function key(){n=n*10+2;return 'x';}function rhs(){n=n*10+3;return 8;}base()[key()]=rhs();n",
    "let init;function seed(f){init=f;return 0;}for(let i=seed(()=>i);i<1;i=i+1){}init()",
    "let x=0;while(true){x=3;break;}x",
]
# Intrinsic object/descriptor regressions shared with the Rust test suite.
cases += [
    "Object.getPrototypeOf({})===Object.prototype && Object.getPrototypeOf(Object.prototype)===null",
    "Object.getPrototypeOf([])===Array.prototype && Object.getPrototypeOf(Array.prototype)===Object.prototype",
    "function f(){};Object.getPrototypeOf(f)===Function.prototype && Object.getPrototypeOf(Function.prototype)===Object.prototype && Function.prototype()===undefined",
    "Object.getPrototypeOf(Object)===Function.prototype && Object.getPrototypeOf(Array)===Function.prototype && Object.getPrototypeOf(Function)===Function.prototype",
    "const node={x:1};const element=Object.create(node);const html=Object.create(element);html.x=2;html.x===2 && node.x===1 && html.hasOwnProperty('x') && !element.hasOwnProperty('x')",
    "const p={x:undefined};const o=Object.create(p);!o.hasOwnProperty('x') && p.hasOwnProperty('x') && Object.getPrototypeOf(o)===p",
    "const o=Object.create(null);o.toString===undefined && Object.getPrototypeOf(o)===null",
    "({}).toString()==='[object Object]' && Object.prototype.toString.call([])==='[object Array]' && Object.prototype.toString.call(null)==='[object Null]'",
    "const d=Object.getOwnPropertyDescriptor(Array.prototype,'push');d.writable && !d.enumerable && d.configurable",
    "const p={};Object.defineProperty(p,'x',{get:function(){return this.value;},set:function(v){this.value=v;}});const o=Object.create(p);o.x=42;o.x===42 && o.value===42 && p.value===undefined && !o.hasOwnProperty('x')",
    "const o={};let seen;Object.defineProperty(o,'x',{get:function(){seen=this;return 7;}});o.x===7 && seen===o",
    "const p={};Object.defineProperty(p,'x',{set:function(v){this.y=v;}});const o=Object.create(p);o.x=4;o.x===undefined && o.y===4",
    "const p={value:7,writable:true};const d=Object.create(p);const o={};Object.defineProperty(o,'x',d);o.x=9;o.x===9",
    "let s='';const d={};Object.defineProperty(d,'enumerable',{get:function(){s=s+'e';return true;}});Object.defineProperty(d,'value',{get:function(){s=s+'v';return 3;}});const o={};Object.defineProperty(o,'x',d);s==='ev' && o.x===3",
    "const o={};Object.defineProperty(o,'x',{value:3});const d=Object.getOwnPropertyDescriptor(o,'x');d.value===3 && !d.writable && !d.enumerable && !d.configurable",
    "const o={};Object.defineProperty(o,'x',{value:NaN});Object.defineProperty(o,'x',{value:NaN})===o",
    "const o={x:1};Object.defineProperty(o,'x',{enumerable:false});o.x=2;const d=Object.getOwnPropertyDescriptor(o,'x');d.value===2 && !d.enumerable && d.writable && d.configurable",
    "const o={};Object.defineProperty(o,'x',{value:1,writable:true});Object.defineProperty(o,'x',{value:2,writable:false});o.x===2",
    "const o={x:1};Object.defineProperty(o,'x',{get:function(){return 4;}});o.x===4 && Object.getOwnPropertyDescriptor(o,'x').value===undefined",
    "const o={};Object.defineProperty(o,'x',{get:function(){return 1;},configurable:true});Object.defineProperty(o,'x',{value:5});o.x===5 && !Object.getOwnPropertyDescriptor(o,'x').writable",
    "const a=[1,2];let n=0;const v={valueOf(){n=n+1;return n===1?4294967296:0;}};Object.defineProperty(a,'length',{value:v});a.length===0 && n===2",
    "const a=[1,2];const v={valueOf(){Object.defineProperty(a,'length',{writable:false});return 2;}};Object.defineProperty(a,'length',{value:v});a.length===2 && !Object.getOwnPropertyDescriptor(a,'length').writable",
    "const a=[1];a.push(2,3)===3 && a.pop()===3 && a.length===2 && a[1]===2",
    "const a=[];a.pop()===undefined && a.length===0 && a.push()===0",
    "const a=[1,2,3,4];const b=a.slice(1,-1);b.length===2 && b[0]===2 && b[1]===3 && a.length===4",
    "const a=[];a.length=4;a[2]=8;const b=a.slice(1,4);b.length===3 && !b.hasOwnProperty('0') && b[1]===8 && !b.hasOwnProperty('2')",
    "const a=[1,2];Object.defineProperty(a,'4',{value:7,configurable:true});a.length===5 && a[4]===7",
    "const a=[1,2,3];Object.defineProperty(a,'length',{value:1});const d=Object.getOwnPropertyDescriptor(a,'length');a[1]===undefined && d.value===1 && d.writable && !d.enumerable && !d.configurable",
    "const a={length:0};Array.prototype.push.call(a,5,6);Array.prototype.pop.call(a)===6 && a.length===1 && !a.hasOwnProperty('1')",
    "function f(a,b){return this.x+a+b;}const o={x:3};f.call(o,4,5)===12 && f.apply(o,[4,5])===12",
    "function f(){return this;}f.call(null)===null && f.apply(undefined,null)===undefined",
    "function f(a,b){return a===undefined && b===undefined;}f.apply(null,[]) && f.call(null)",
    "function f(a,b){return a+b;}f.apply(null,{0:2,1:3,length:2})===5",
    "const f=()=>this;f.call(null)===globalThis",
]

for source in cases:
    # Compare Script completion values with strict execution in an isolated realm.
    js = "const vm=require('node:vm');const r=vm.runInNewContext(" + json.dumps('"use strict";'+source) + ");if(r!==undefined)console.log(String(r));"
    expected = subprocess.check_output(["node", "-e", js], text=True)
    actual = subprocess.check_output([str(runner), "-e", source], text=True)
    if actual != expected:
        raise AssertionError(f"{source}\nNode={expected!r}\nSunJS={actual!r}")
print(f"Node/V8 differential: {len(cases)} selected cases PASS")

for name in ["arithmetic", "counter", "loop", "method", "array", "native"]:
    path=root / "sunlight-js/examples" / (name+".js")
    wrapper="const vm=require('node:vm');vm.runInNewContext("+json.dumps('"use strict";'+path.read_text())+",{print:(...a)=>console.log(a.map(String).join(' ')),assert:x=>{if(!x)throw Error('assert');}});"
    expected=subprocess.check_output(["node","-e",wrapper],text=True)
    actual=subprocess.check_output([str(runner),str(path)],text=True)
    if expected!=actual: raise AssertionError(f"example {name}: {expected!r} != {actual!r}")
print("Six runnable examples agree with Node/V8")
