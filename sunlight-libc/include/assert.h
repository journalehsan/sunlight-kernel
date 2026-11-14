/* assert must be re-evaluated on every inclusion after NDEBUG changes. */
#ifndef SUNLIGHT_ASSERT_DECLARED
#define SUNLIGHT_ASSERT_DECLARED
#ifdef __cplusplus
extern "C" [[noreturn]] void __sunlight_assert_fail(const char *, const char *, unsigned int, const char *);
#else
_Noreturn void __sunlight_assert_fail(const char *, const char *, unsigned int, const char *);
#endif
#endif
#undef assert
#ifdef NDEBUG
#define assert(expression) ((void)0)
#else
#define assert(expression) ((expression) ? (void)0 : __sunlight_assert_fail(#expression, __FILE__, __LINE__, __func__))
#endif
#if !defined(__cplusplus)
#undef static_assert
#define static_assert _Static_assert
#endif
