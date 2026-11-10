/* Compile-only C11/IEEE floating-point smoke test; no libc or startup. */
_Static_assert(sizeof(void *) == 8, "64-bit pointers required");
_Static_assert(sizeof(double) == 8, "64-bit double required");
double audit_sqrt_input(double x) { return x * x + 1.0; }
