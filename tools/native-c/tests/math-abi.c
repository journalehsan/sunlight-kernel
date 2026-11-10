#ifdef REVERSE_INCLUDES
#include <float.h>
#include <stddef.h>
#endif
#include <math.h>
#include <math.h>
#ifndef REVERSE_INCLUDES
#include <stddef.h>
#include <float.h>
#endif
#ifdef __cplusplus
template<class A, class B> struct same { static const bool value = false; };
template<class A> struct same<A, A> { static const bool value = true; };
#define TYPE(expr, type) static_assert((same<decltype(expr), type>::value), #expr)
#else
#define TYPE(expr, type) _Static_assert(_Generic((expr), type: 1, default: 0), #expr)
#endif
TYPE(&sqrt, double (*)(double));
TYPE(&sqrtf, float (*)(float));
TYPE(&fabs, double (*)(double));
TYPE(&fabsf, float (*)(float));
TYPE((float_t)0, float);
TYPE((double_t)0, double);
TYPE(INFINITY, float);
TYPE(NAN, float);
TYPE(HUGE_VAL, double);
TYPE(HUGE_VALF, float);
TYPE(HUGE_VALL, long double);
#if FLT_EVAL_METHOD != 0
#error Native SSE2 header requires direct float/double evaluation
#endif
double abi_calls(double x) { return sqrt(x) + fabs(x) + sqrtf((float)x) + fabsf((float)x); }
