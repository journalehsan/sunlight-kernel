#include <math.h>
#include <stdint.h>

/* Volatile calls force the archive exports, including under optimization. */
static double (*volatile root)(double) = sqrt;
static float (*volatile rootf)(float) = sqrtf;
static double (*volatile absolute)(double) = fabs;
static float (*volatile absolutef)(float) = fabsf;
static uint64_t bits(double x) { union { double f; uint64_t u; } v = { .f = x }; return v.u; }
static uint32_t bitsf(float x) { union { float f; uint32_t u; } v = { .f = x }; return v.u; }
static double value(uint64_t x) { union { uint64_t u; double f; } v = { .u = x }; return v.f; }
static float valuef(uint32_t x) { union { uint32_t u; float f; } v = { .u = x }; return v.f; }
static int nan64(double x) { return (bits(x) & UINT64_C(0x7fffffffffffffff)) > UINT64_C(0x7ff0000000000000); }
static int nan32(float x) { return (bitsf(x) & UINT32_C(0x7fffffff)) > UINT32_C(0x7f800000); }

int main(void) {
    if (bits((double)INFINITY) != UINT64_C(0x7ff0000000000000) ||
        bits(HUGE_VAL) != UINT64_C(0x7ff0000000000000) ||
        bitsf(HUGE_VALF) != UINT32_C(0x7f800000) || !nan32(NAN) ||
        HUGE_VALL <= 0.0L) return 1;
    if (bits(root(-0.0)) != UINT64_C(0x8000000000000000) || bits(root(0.0)) ||
        bitsf(rootf(-0.0f)) != UINT32_C(0x80000000) || bitsf(rootf(0.0f))) return 2;
    if (root(4.0) != 2.0 || rootf(4.0f) != 2.0f ||
        bits(root(2.0)) != UINT64_C(0x3ff6a09e667f3bcd) ||
        bitsf(rootf(2.0f)) != UINT32_C(0x3fb504f3)) return 3;
    if (root(HUGE_VAL) != HUGE_VAL || rootf(INFINITY) != INFINITY ||
        !nan64(root(-1.0)) || !nan32(rootf(-1.0f)) ||
        !nan64(root(-HUGE_VAL)) || !nan32(rootf(-INFINITY)) ||
        !nan64(root((double)NAN)) || !nan32(rootf(NAN))) return 4;
    if (bits(root(value(1))) != UINT64_C(0x1e60000000000000) ||
        bitsf(rootf(valuef(1))) != UINT32_C(0x1a3504f3) ||
        bits(root(value(UINT64_C(0x7fefffffffffffff)))) != UINT64_C(0x5fefffffffffffff) ||
        bitsf(rootf(valuef(UINT32_C(0x7f7fffff)))) != UINT32_C(0x5f7fffff)) return 5;
    if (bits(absolute(-0.0)) || bitsf(absolutef(-0.0f)) ||
        absolute(-3.5) != 3.5 || absolutef(-3.5f) != 3.5f ||
        absolute(-HUGE_VAL) != HUGE_VAL || absolutef(-INFINITY) != INFINITY) return 6;
    if (bits(absolute(value(UINT64_C(0xfff8000000001234)))) != UINT64_C(0x7ff8000000001234) ||
        bitsf(absolutef(valuef(UINT32_C(0xffc01234)))) != UINT32_C(0x7fc01234) ||
        bits(absolute(value(UINT64_C(0x8000000000000001)))) != 1 ||
        bitsf(absolutef(valuef(UINT32_C(0x80000001)))) != 1) return 7;
    return 0;
}
