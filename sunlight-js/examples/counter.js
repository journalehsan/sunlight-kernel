function counter() { let n = 0; return () => { n = n + 1; return n; }; }
const next = counter();
print(next(), next(), next());
