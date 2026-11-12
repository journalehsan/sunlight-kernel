#include <new>
extern "C" void* malloc(__SIZE_TYPE__) noexcept;
void* operator new(__SIZE_TYPE__ n) {
    if (void* p = malloc(n ? n : 1)) return p;
    throw std::bad_alloc();
}
void* operator new[](__SIZE_TYPE__ n) { return ::operator new(n); }
