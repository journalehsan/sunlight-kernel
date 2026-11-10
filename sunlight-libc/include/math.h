#ifndef SUNLIGHT_MATH_H
#define SUNLIGHT_MATH_H

/* Native x86-64/SSE2 subset (FLT_EVAL_METHOD == 0).
 * No errno/fenv or full C libm contract is advertised. */
typedef float float_t;
typedef double double_t;

#define HUGE_VAL (__builtin_huge_val())
#define HUGE_VALF (__builtin_huge_valf())
#define HUGE_VALL (__builtin_huge_vall())
#define INFINITY (__builtin_inff())
#define NAN (__builtin_nanf(""))

#ifdef __cplusplus
extern "C" {
#endif
double sqrt(double x);
float sqrtf(float x);
double fabs(double x);
float fabsf(float x);
#ifdef __cplusplus
}
#endif
#endif
