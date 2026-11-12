#include <stdio.h>
#include <stdarg.h>
using size_t = __SIZE_TYPE__;
extern "C" void free(void*) noexcept;
extern "C" [[noreturn]] void sl_std_exit(int) noexcept;
extern "C" [[noreturn]] void __cxa_pure_virtual() { sl_std_exit(134); }
extern "C" [[noreturn]] void __cxa_deleted_virtual() { sl_std_exit(134); }
// Compiler-emitted deleting destructors use the same native allocator as
// exception message storage. No allocation operator is supplied by this layer.
void operator delete(void* p) noexcept { free(p); }
void operator delete(void* p, size_t) noexcept { free(p); }
void operator delete[](void* p) noexcept { free(p); }
void operator delete[](void* p, size_t) noexcept { free(p); }
extern "C" [[noreturn]] void __abort_message(const char* format, ...) {
    va_list args;
    va_start(args, format);
    vfprintf(stderr, format, args);
    va_end(args);
    sl_std_exit(134);
}
