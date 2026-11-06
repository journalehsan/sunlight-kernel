# SunJS and the controlled Golden Fish DOM fixture

SunJS is an independent Rust 2021, native-std tree interpreter for an explicitly
limited strict ECMAScript language subset. It is **not a conforming ECMAScript
implementation**. Rabbit navigation never executes inline or external scripts.
An opt-in local fixture harness can bind SunJS to Golden Fish and repaint a
retained browser viewport, as described below.
There are no new third-party dependencies, JIT, filesystem, network, environment,
process, DOM, Tokio, or renderer dependencies in `sunlight-js`.

## Running and embedding

```sh
cargo test -p sunlight-js -p sunlight-js-cli --target x86_64-unknown-linux-gnu
cargo build -p sunlight-js-cli --target x86_64-unknown-linux-gnu
./target/x86_64-unknown-linux-gnu/debug/sunjs sunlight-js/examples/counter.js
./target/x86_64-unknown-linux-gnu/debug/sunjs --smoke
python3 tools/test_sunjs_compare.py
bash tools/build_sunjs.sh
bash tools/test_sunjs.sh
```

`sunjs FILE` reads UTF-8 source in the CLI host. `sunjs -e SOURCE` evaluates a
command, and `--smoke` evaluates the embedded deterministic guest fixture. The
CLI explicitly installs `print` and `assert`; neither exists in a fresh engine.
No REPL is provided yet. The native binary is
`target/x86_64-sunlightos/release/sunjs`. The guest gate follows the existing
native-std probe and network gate workflows: opt-in kernel embedded ELF, real TTY
login through QMP key events, isolated ISO, two QEMU CPUs, serial verdict and successful process
exit. It does not open the user's disk image. The `sunjs_probe` kernel feature
is disabled by default.

```rust
use sunlight_js::{Context, Error, ErrorKind, Limits, Value};
fn main() -> Result<(), Error> {
let mut js = Context::new(Limits { fuel: 20_000, call_depth: 32,
                                ..Limits::default() });
js.register_native("assert", |_, _, args| {
    if args.first().is_some_and(Value::truthy) { Ok(Value::Undefined) }
    else { Err(Error::new(ErrorKind::Exception, "assertion failed")) }
})?;
let host = js.create_object(Some(js.object_prototype()))?;
js.define_data(host, "answer", Value::Number(42.0), false)?;
js.set_global("host", Value::Object(host))?;
let result = js.eval("assert(host.answer === 42); host.answer;", "example.js")?;
assert_eq!(result, Value::Number(42.0));
js.close()?;
Ok(())
}
```

Native accessors use `native_function` and `define_accessor`; their getters and
setters receive the original JS receiver. Callbacks return `JsResult<Value>`.
Use `Error::new` to return a structured JS error, not a Rust panic. Native Rust
panics and host OOM are not recoverable JS exceptions on the aborting native
runtime. A callback is trusted Rust: fuel cannot interrupt a callback that
blocks or loops itself. Hosts must bound their own work. Native callbacks may
reenter evaluation; nested evaluation shares the outer fuel. Closing an active
context fails rather than invalidating the interpreter's active state.

Errors include kind, source name, UTF-8 byte offset, one-based line/Unicode
scalar column, message, and an optional thrown JS value. String indexes and
lengths are separately measured in UTF-16 code units. Console display replaces
lone surrogates for output only; runtime strings preserve them.

## Supported and unsupported matrix

| Area | Implemented | Explicitly outside this milestone |
| --- | --- | --- |
| Bindings | `let`, `const`, multiple declarators, lexical blocks, shadowing, TDZ, immutable const binding | `var`, destructuring, dynamic scope, `with` |
| Values | binary64 numbers, UTF-16 strings, booleans, null, undefined, object identities | BigInt, Symbol, boxed primitives |
| Literals | decimal/exponent numbers, single/double quoted strings, `\uXXXX`, `\xXX`, basic escapes, arrays, data objects, concise methods | hex/binary/octal source numbers, legacy leading-zero numbers/escapes, templates, regex, array holes, spread, computed/shorthand properties, surrogate literal property names, JS getters/setters, `__proto__` literal |
| Operators | `+ - * / %`, unary `+ - !`, `< <= > >=`, `=== !==`, `=`, `&& || ??`, ternary, grouping, member/index access, calls | coercive equality, bitwise, exponentiation, update/compound assignment, comma operator, `typeof`, `delete`, `void`, `in`, `instanceof`, optional chaining |
| Control | blocks, `if/else`, `while`, three-clause `for`, unlabelled break/continue, return, throw | do/while, for/in, for/of, switch, labels, try/catch/finally |
| Functions | declarations, named/anonymous expressions, plain-parameter arrows, closures, recursion, strict ordinary `this`, lexical arrow `this`, `Function.prototype.call/apply` | defaults/rest/destructured parameters, arguments object, constructors/new, classes/super, generators, async/await |
| Objects | intrinsic Object/Function/Array chains, `Object.create`, `getPrototypeOf`, `defineProperty`, `getOwnPropertyDescriptor`, data/accessor descriptors, inherited receiver binding and shadowing, native accessors, `hasOwnProperty`, `toString` | prototype mutation, deletion syntax, enumeration, Proxy, weak references |
| Arrays | sparse indices, `push`, `pop`, `slice`, index/length descriptors, length growth/truncation/range checks | other Array methods (including array string conversion), iterators, species |
| Strings | UTF-16 equality/ordering/concatenation, code-unit indexing and length | String standard methods, Unicode identifiers/identifier escapes, braced Unicode escapes, escaped source line continuations |
| Runtime | isolated contexts, structured errors, callbacks, host objects, controlled fuel/depth/arena/source/string limits | modules/import loading, eval/Function constructors, standard library, event loop |

Unsupported grammar fails parsing before any statement executes. No unsupported
statement is skipped. Legacy `__proto__` reads/writes and object literal syntax
remain rejected; use internal prototype links through `Object.create` and
`Object.getPrototypeOf`. `Object`, `Array`, and `Function` expose their intrinsic
prototypes. `Object()` creates an ordinary object or returns an existing object;
`Array()` creates an array. Dynamic `Function()` compilation remains Unsupported,
as do constructors with `new`. Uninstalled globals such as `Math`, `JSON`,
`Promise`, and `eval` raise Reference errors. Missing properties are undefined.
Object-to-primitive coercion consults inherited `valueOf`/`toString` methods;
ordinary objects default to `[object Object]`. Failure to obtain a primitive
raises Type. Primitive arithmetic/string conversion is supported.
Numeric string conversion covers decimal and Infinity, plus hex/binary/octal
integers through 128 bits. Larger valid nondecimal strings raise Unsupported.

All source executes under strict binding and property-write rules, regardless
of directives. Missing identifiers throw; there is no implicit global creation.
Bare ordinary calls receive undefined `this`; methods receive their base,
including parenthesized method references. Arrows capture creation-time `this`.
Top-level `this` is the context's global object (`globalThis`). Top-level lexical
bindings are retained between evaluations and are not properties on that object.
Top-level function declarations are also available through the global object;
reads and assignments through either path access the same binding.
Repeated declarations in the same lexical environment, including function
declarations, are rejected. Function declarations in blocks are initialized at
block entry; declarations directly under if/loops require braces (no Annex B).
Parameters cannot duplicate one another or direct lexical body bindings.
Automatic semicolon insertion handles line boundaries, block/end boundaries,
and the restricted return/throw line terminators; unsupported continuation
syntax is diagnosed rather than silently split. Mixing `??` and `&&`/`||`
requires parentheses. Decimal numbers use IEEE-754 binary64, including NaN,
infinities, signed zero, and ordinary primitive coercions. Logical operators
return operands and short-circuit. References are resolved before right-hand
sides; member bases/key expressions, arguments, and operands evaluate left to
right. Computed assignment keys retain their value and defer ToPropertyKey
coercion until after the RHS (and after rejecting a null/undefined base), as in
[ECMAScript property references](https://tc39.es/ecma262/multipage/ecmascript-language-expressions.html#sec-runtime-semantics-evaluatepropertyaccesswithexpressionkey).

## Architecture and memory ownership

`lexer.rs` owns tokens and locations; `parser.rs` constructs owned statements and
expressions from `ast.rs`. Pratt expression parsing is separate from statement
parsing. A future module entry point can add a distinct goal and module AST
without adding I/O to parsing. `runtime.rs` implements lexical environments,
statement execution, expression evaluation, function calls, and explicit
Normal/Return/Break/Continue completions. `JsResult` is the exceptional completion
channel. `runtime/objects.rs` implements property/prototype lookup and assignment;
`runtime/operators.rs` implements primitive operations; `runtime/host.rs` exposes
native bindings and descriptors. `value.rs` defines public values/errors/handles.
The CLI is a separate crate and capability host. The minimal DOM adapter lives
in Rabbit behind `js-dom-fixture`; Golden Fish and Rabbit remain outside the
engine dependency graph.

Object/function handles contain a context identity, arena index and generation, with no
exposed mutable internals. Handles from a different or closed context fail.
Prototype links are chosen at allocation from already-existing objects. There
is no prototype mutation, so prototype cycles cannot form. Ordinary objects
inherit Object.prototype; arrays inherit Array.prototype; script and native
functions inherit the callable Function.prototype. Both Array.prototype (itself
an empty array) and Function.prototype inherit Object.prototype, whose prototype
is null. Intrinsic constructors inherit Function.prototype. Built-in methods
are writable, configurable and non-enumerable.

Properties store either a data value plus writable/enumerable/configurable flags,
or optional getter/setter functions plus enumerable/configurable flags. Reading
or writing an inherited accessor calls it with the original receiver. Inherited
writable data properties are shadowed; readonly data and missing setters throw
Type on assignment. Getters without setters and setters without getters are
supported. Object literal getter/setter syntax remains outside the grammar;
JavaScript registers callbacks through `Object.defineProperty`.

`Object.defineProperty` accepts partial descriptors. Omitted fields retain their
current values on redefinition, or default to undefined/false for new properties.
Data/accessor mixtures and non-callable callbacks throw Type. Configurable
properties can change kind. Non-configurable properties cannot change kind,
enumerability or callbacks; non-writable data values require SameValue (including
NaN and signed zero). `Object.getOwnPropertyDescriptor` exposes flags and callbacks.
Array length is a non-enumerable, non-configurable data property; index definitions
grow it, readonly length blocks growth, and shrinking deletes configurable indices
in descending order. A non-configurable element blocks truncation at that index.
`slice` preserves holes and reads inherited indices. `call`/`apply` retain strict
receivers, and apply accepts bounded array-like argument objects.

The Rust `PropertyDescriptor` and `Context::define_property` API expose the same
partial descriptor rules. `define_data` and `define_accessor` remain convenience
APIs for enumerable, non-configurable host properties. Create native callbacks
with `native_function`; callbacks receive the original receiver even when the
accessor is on an ancestor prototype. For example:

```rust
use sunlight_js::{Context, Error, Value};
fn main() -> Result<(), Error> {
let mut js = Context::default();
let node = js.create_object(Some(js.object_prototype()))?;
let element = js.create_object(Some(node))?;
let html_element = js.create_object(Some(element))?;
let getter = js.native_function(|js, receiver, _| {
    js.get_property(receiver, "storedText")
})?;
let setter = js.native_function(|js, receiver, args| {
    js.set_property(receiver, "storedText", args[0].clone())?;
    Ok(Value::Undefined)
})?;
js.define_accessor(node, "textContent", Some(getter), Some(setter))?;
js.set_global("element", Value::Object(html_element))?;
assert_eq!(js.eval("element.textContent='hello';element.textContent", "host.js")?,
           Value::string("hello"));
Ok(())
}
```

Prototype mutation, Object.create's second descriptor-map argument, global-binding
descriptor redefinition, primitive boxing, symbols, enumeration, and the rest of
the standard library remain explicit limitations. Intrinsic installation adds a
fixed context allocation cost; it is counted in arena usage even when the caller
sets a limit below the bootstrap size. Subsequent allocations obey that limit.

The context owns a safe, non-moving tracing heap for objects (including arrays
and functions) and lexical environments. Each slot has a mark flag, cell type,
generation and free-list link. Sweeping drops unmarked payloads with normal Rust
destructors, increments generations before reuse, and clears survivor marks.
Stale, foreign and closed-context handles produce Reference errors. There is no
unsafe code or third-party collector dependency.

`Context::collect_garbage()` performs a deterministic stop-the-world mark/sweep
and returns `GcStats`: reclaimed/live objects and environments, live entry count,
slot high-water counts and cumulative collections. `arena_usage()` now reports
live cells. An explicit gray worklist traces prototypes, data/accessor values,
function environments, lexical `this`, parent environments and bindings without
recursing on the Rust stack. Globals and intrinsic prototypes are permanent
roots. Interpreter root frames retain active environments, receivers, arguments,
partial literals, operands, completions and thrown values across nested calls,
accessors and reentrant evaluation. Loop/list safe points release completed
iteration temporaries. Native allocations remain rooted until that call returns.

Automatic collection runs at fuel-charge safe points after 256 cell allocations
by default; `set_gc_threshold(n)` configures that interval (zero stresses every
safe point). Allocation itself never collects halfway through a mutation. A
single operation can therefore hit the hard arena limit before a collection
point; hosts may manually collect before retrying. Collection work is synchronous
and is not charged against JS fuel. It scales with arena high-water slots and
reachable edges. Live graphs remain subject to the existing limits.

Host-held `Value`/`Handle` copies are **not roots**. Pin them before any execution
or explicit collection that must preserve them:

```rust
use sunlight_js::{Context, Error};
fn main() -> Result<(), Error> {
    let mut js = Context::default();
    let value = js.eval("({answer:42})", "host")?;
    let root = js.register_root(value)?;
    js.collect_garbage()?;
    assert_eq!(js.get_property(root.value(), "answer")?.display(), "42");
    drop(root); // Last HostRoot clone releases the pin.
    js.collect_garbage()?;
    Ok(())
}
```

`HostRoot` owns no context and does not prevent close/drop. Its registration is
weak on the context side. Register returned objects and object-valued error
payloads before the next execution/collection. Rust callback captures are opaque:
use `native_function_with_roots(captures, callback)` to declare all captured JS
references as edges owned by that native function. This also permits unreachable
native/function/object capture cycles to be reclaimed. Use explicit pins for
external caches. Do not capture strong Context ownership in a callback.

Arrays store dynamic elements in their managed object's property map. UTF-16
strings remain owned `Vec<u16>` values for public API compatibility: they are
acyclic buffers, freed by Rust when their containing values/cells die, and cloned
by value. AST/source buffers use acyclic Rc ownership and are released with the
last function referencing them. Strings and array backing maps are **not separate
GC cells**, and string copies, property keys and AST storage are not counted in
cell statistics. This is not a byte budget or a compacting/incremental collector.
Slot capacity retains the high-water mark for reuse; sweeping releases payloads
but does not promise lower process RSS. Headers and root worklists add memory
and tracing adds execution overhead; no allocation/throughput benchmark is claimed.

Defaults: 100,000 evaluation fuel, 64 calls, 100,000 arena entries, 256 KiB source,
1 Mi UTF-16 units per string. Parsing also caps nesting (96) and individual
expression chains (64). These are resource limits, not syntax extensions. Fuel
charges statements, expressions, calls, loop iterations and prototype traversal.
Fuel resets on an outer eval, not nested eval. Unreachable arena payloads are reclaimed at collection points. Limits bound engine work, not arbitrary native callbacks, total process
RSS, allocator failure or privileged host capabilities. There is no unsafe code
in the engine or CLI.

## Controlled Golden Fish DOM adapter

`rappid-rabbit/src/dom_fixture.rs` is an internal crate bridge, available only
with the **non-default** `js-dom-fixture` feature. `sunlight-js` is an optional
Rabbit dependency, with no new third-party dependency. Browser navigation,
script resource discovery, loading and event handlers never invoke this bridge,
even in a build with the fixture feature. Opening the fixture in Rabbit renders
`Old`; only the explicit harness executes its script. No URL, file extension,
or network response can opt into execution.

```sh
# Deterministic DOM/arena/paint regressions
cargo test -p rappid-rabbit --features js-dom-fixture \
  --test js_dom_fixture --target x86_64-unknown-linux-gnu
# Visual harness: compiled-in local HTML only; no arbitrary input or networking
cargo run -p rappid-rabbit --features js-dom-fixture \
  --example js_dom_smoke --target x86_64-unknown-linux-gnu
```

The visual example explicitly selects the inline script in
`rappid-rabbit/tests/fixtures/js_dom_smoke.html`. It writes the real
`DocumentCanvas` viewport to `target/js-dom-smoke/before.ppm` and `after.ppm`:
`<div id="counter">Old</div>` becomes `New Text`. The integration test checks
DOM text, layout arena geometry, retained scene text, dirty paint regions,
changed viewport pixels, and the unchanged pre-mutation worker snapshot.

The Rust entry point is
`run_fixture_script(&mut DocumentRenderState, source, source_name, measurer)`.
Call it only with explicitly selected, trusted fixture source. Each call owns a
fresh bounded `Context` and an `Arc<Document>` lease. It installs:

| Binding | Minimal contract |
| --- | --- |
| `window` | The context global object, identical to `globalThis`; `window.document === document` |
| `document.getElementById(id)` | First connected element in tree order, exact case-sensitive ID; missing/empty IDs yield `null` |
| `element.textContent` | Native prototype accessor; getter concatenates descendant text and excludes comments; setter replaces children with plain text |

This intentionally small API accepts **strings only** for IDs and text.
Numbers, objects, null/undefined and lone UTF-16 surrogates raise Type errors;
there is no object coercion or lossy UTF-8 conversion. This restriction is not
full WebIDL/DOM string conversion. No HTML parsing, events, timers, external
script loading, browser event loop, or shell capabilities are installed.

A private bidirectional `NodeId`/SunJS handle map, with explicit `HostRoot` pins, preserves wrapper identity
within the run. Receiver validation uses registered handles, never script-visible
IDs or pointers; borrowed accessors and objects inheriting from a wrapper cannot
forge element identity. Handles include SunJS context identity. No handles or
callbacks escape the turn, so no wrapper can be reused across navigation or runs.
Thrown object payloads are removed from returned errors before context teardown;
primitive payloads and source diagnostics are retained. There are no raw
references or unsafe code in the bridge.

Golden Fish's `get_element_by_id`, `element_text_content` and
`set_element_text_content` methods maintain both child and parent arenas.
Replaced subtrees are detached, remain readable through existing wrappers, and
stop participating in document lookup/layout. IDs are never recycled. A sole
text child is reused; unchanged leaf writes avoid invalidation. Text writes are
synchronous, so subsequent getters and lookups observe them immediately.

The setter marks the owned DOM lease dirty. At normal or exceptional script
exit, the host drops the context, publishes the copy-on-write DOM, increments
`dom_revision`, recomputes embedded/inline styles for the current viewport, and
runs the existing two-pass arena layout and scene paint pipeline **once**.
This fixture path supports embedded styles only; it never fetches linked CSS
and does not retain externally supplied stylesheet lists.
`FixtureRun.repaint_requested` tells the owner to paint the prepared scene and
`last_patch` dirty regions, including when `evaluation` contains an error.
Earlier writes persist after exceptions or resource exhaustion. The visual
harness/test honors this notification by drawing `DocumentCanvas` to pixels.
Native callbacks never directly draw widgets. Rabbit's resize completion check
now requires matching DOM revision as well as navigation/image/viewport state,
so an older worker snapshot cannot overwrite post-mutation output.

In addition to the default SunJS limits, each run caps the DOM at 4,096 allocated
nodes (including detached nodes), initially allocated text/comments plus
cumulative setter text at 256 KiB, setter calls at 256, and native traversal work
at 262,144 work units. Traversals charge nodes, lookups conservatively charge
nodes times ID bytes, and string conversion/copying charges input UTF-16 units
or output UTF-8 bytes. A setter at the node cap is conservatively rejected even
if it could reuse a text child. These bounds cover native callback work that JS
fuel alone cannot interrupt. DOM ownership is exclusive during the synchronous
turn; reflow snapshots keep their old immutable DOM.

Before any external scripts can run, a separate design is still required for
origins, loading/CSP, navigation, scheduling, lifecycle and events. Tracing GC and explicit callback roots support memory reclamation; browser execution
remains disabled by this fixture feature.

## Validation scope and next milestone

The GC change passes all 17 engine and six object-model tests, five new GC tests,
82 Node/V8 comparisons and six examples, plus all ten DOM fixture tests and 171
Rabbit library tests (including the new cached/detached-wrapper GC test), on
`x86_64-unknown-linux-gnu`. The same suites also passed with the default GC
threshold temporarily set to zero, collecting at every fuel-charge safe point;
the shipped default remains 256. Strict engine Clippy (`--all-targets -- -D
warnings`) passes. Rabbit builds retain existing dependency/CSS warnings.
The GC tests reclaim object/closure cycles back to baseline for 200 rounds,
check native capture destructor cleanup and stale-generation rejection, trace a
20,000-edge cycle, and run 2,000 iterations with a 1,500-entry arena limit while
keeping both arena slot counts below 100. These are deterministic cell-growth
assertions, not process allocation or latency benchmarks. No QEMU/guest boot gate
or full-workspace validation was run for this change.


The fixture bridge was validated on the explicit Linux host target: all 17
original engine tests and six object-model tests, 82 Node/V8 comparisons plus
six examples, 27 Golden Fish unit tests, 170 Rabbit library tests, 15 Rabbit
binary tests, and 284 shared UI unit tests passed. Ten DOM fixture tests
additionally check exceptional exit, invalid receivers, detached wrappers, host work limits,
and context teardown. Browser/document/image integration tests also passed.
The visual example produced `New Text` in the viewport. The opt-in Rabbit library
also passed `bash tools/build_native_std.sh check -p rappid-rabbit --lib
--features js-dom-fixture`; this is a native-std compilation check, not a guest
boot or browser GUI run. The bridge requires std and is not a bare-metal kernel
binding; the workspace default target remains `x86_64-unknown-none`.

The broader host test command reached an existing failing UI doctest in
`sunlight-ui/horizon.rs:5` (an indented window-control diagram interpreted as Rust).
All selected unit/integration suites passed; a full doctest pass is not claimed.
Rabbit Clippy with `--no-deps` completed with existing warnings and no diagnostics
in the added bridge/fixture files. Strict Golden Fish Clippy remains blocked by
existing diagnostics in Golden Fish and `vendor/tl`. Formatting and diff checks
pass. No full workspace or QEMU boot gate was run for this adapter.

See `javascript-validation.md` for the earlier engine milestone commands/results.
Targeted tests and selected Node/V8 comparisons are development checks, not Test262 or full
ECMAScript conformance. No Test262 suite was imported or executed.

The intrinsic object layer and descriptor rules now support the minimal DOM
wrapper chain used by the controlled fixture. The tracing collector now reclaims unreachable cycles; the fixture identity cache
intentionally pins wrappers for its bounded turn. Keep module loading and browser
script scheduling separate from memory management.
