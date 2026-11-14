#ifndef SUNLIGHT_MATH_H
#define SUNLIGHT_MATH_H

/* Native x86-64/SSE2 subset (FLT_EVAL_METHOD == 0).
 * Targeted additions use pinned Rust libm. Domain/pole/overflow set errno;
 * underflow errno and a public fenv API are not supported. No full-libm claim.
 * long double is x87 extended (64-bit mantissa, 16-byte storage): its variants
 * are unsupported, never silently narrowed to double. */
/* A global math_errhandling contract is not advertised for the older subset. */
#define signbit(x) __builtin_signbit(x)
#define isnan(x) __builtin_isnan(x)
#define isinf(x) __builtin_isinf(x)
#define isfinite(x) __builtin_isfinite(x)
#define isnormal(x) __builtin_isnormal(x)
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
double atanh(double x);
float atanhf(float x);
double frexp(double x, int *exponent);
float frexpf(float x, int *exponent);
double modf(double x, double *integral);
float modff(float x, float *integral);
double lgamma(double x);
float lgammaf(float x);
/* POSIX signgam is shared storage; concurrent lgamma calls require external
 * synchronization. GNU reentrant variants do not change signgam. */
extern int signgam;
#if defined(_GNU_SOURCE)
double lgamma_r(double x, int *sign);
float lgammaf_r(float x, int *sign);
#endif
/* Existing compiler-builtins exports, now declared for CPython longobject.
 * These older functions have no errno guarantee. */
/* Existing native compiler-builtins exports. As with log/sqrt below, these
 * older exports have no errno guarantee; no long-double ABI is advertised. */
double atan2(double y, double x);
double ceil(double x);
double copysign(double x, double y);
double cos(double x);
double exp(double x);
double floor(double x);
double fmod(double x, double y);
double hypot(double x, double y);
double pow(double x, double y);
double round(double x);
double sin(double x);
double ldexp(double x, int exponent);
double log(double x);
double sqrt(double x);
float sqrtf(float x);
double fabs(double x);
float fabsf(float x);
#ifdef __cplusplus
}
#endif
#endif
