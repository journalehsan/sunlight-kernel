#ifndef SUNLIGHT_ABI_ASSERT
#define SUNLIGHT_ABI_ASSERT
#ifdef __cplusplus
extern "C" [[noreturn]] void sl_std_exit(int) noexcept;
#define assert(e) ((e) ? void(0) : sl_std_exit(134))
#else
extern void sl_std_exit(int) __attribute__((noreturn));
#define assert(e) ((e) ? (void)0 : sl_std_exit(134))
#endif
#endif
