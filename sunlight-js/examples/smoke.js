function counter() { let n = 0; return () => { n = n + 1; return n; }; }
const next = counter();
assert(next() === 1 && next() === 2);
const functions = [];
for (let i = 0; i < 3; i = i + 1) { functions[i] = () => i; }
assert(functions[0]() === 0 && functions[2]() === 2);
const object = { x: 40, add(n) { return this.x + n; } };
assert(object.add(2) === 42);
assert("😀".length === 2);
print("[SUNJS] native smoke PASS");
