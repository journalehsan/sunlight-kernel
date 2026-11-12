#include <exception>
#include "cxa_exception.h"
#include "cxa_handlers.h"
using size_t = __SIZE_TYPE__;
extern "C" void* malloc(size_t) noexcept;
extern "C" void free(void*) noexcept;
extern "C" [[noreturn]] void sl_std_exit(int) noexcept;
extern "C" [[noreturn]] void __sl_cxx_abort() noexcept { sl_std_exit(134); }
namespace std {
terminate_handler get_terminate() noexcept { return __sl_cxx_abort; }
unexpected_handler get_unexpected() noexcept { return __sl_cxx_abort; }
void __terminate(terminate_handler handler) noexcept {
    try { handler(); } catch (...) { __sl_cxx_abort(); }
    __sl_cxx_abort();
}
void __unexpected(unexpected_handler handler) {
    handler();
    __sl_cxx_abort();
}
[[noreturn]] void terminate() noexcept { __sl_cxx_abort(); }
}
namespace __cxxabiv1 {
// Trivial TLS only: native variant-II TLS provides one independent catch
// stack per thread, without requiring thread destructor registration.
static thread_local __cxa_eh_globals globals;
extern "C" __cxa_eh_globals* __cxa_get_globals() { return &globals; }
extern "C" __cxa_eh_globals* __cxa_get_globals_fast() { return &globals; }
// Native malloc guarantees 16-byte alignment, matching _Unwind_Exception.
// No emergency pool: allocation exhaustion is handled by upstream terminate.
void* __aligned_malloc_with_fallback(size_t n) { return malloc(n); }
void __aligned_free_with_fallback(void* p) { free(p); }
}
