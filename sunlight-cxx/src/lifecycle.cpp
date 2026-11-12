#include <exception>
#include <typeinfo>
#include <pthread.h>
using size_t = __SIZE_TYPE__;
extern "C" void* malloc(size_t) noexcept;
extern "C" void free(void*) noexcept;
namespace {
struct Destructor { void (*function)(void*); void* argument; Destructor* next; };
pthread_mutex_t mutex = PTHREAD_MUTEX_INITIALIZER;
Destructor* process_destructors;
thread_local Destructor* thread_destructors;
Destructor* make(void (*f)(void*), void* arg, Destructor* next) noexcept {
    auto* p = static_cast<Destructor*>(malloc(sizeof(Destructor)));
    if (!p) std::terminate();
    *p = {f, arg, next};
    return p;
}
}
extern "C" {
__attribute__((visibility("hidden"))) void* __dso_handle = &__dso_handle;
int __cxa_atexit(void (*function)(void*), void* argument, void*) noexcept {
    if (pthread_mutex_lock(&mutex)) std::terminate();
    process_destructors = make(function, argument, process_destructors);
    if (pthread_mutex_unlock(&mutex)) std::terminate();
    return 0;
}
int __cxa_thread_atexit(void (*function)(void*), void* argument, void*) noexcept {
    thread_destructors = make(function, argument, thread_destructors);
    return 0;
}
void __sl_cxx_tls_finalize() noexcept {
    while (thread_destructors) {
        Destructor* p = thread_destructors;
        thread_destructors = p->next;
        auto function = p->function;
        void* argument = p->argument;
        free(p);
        try { function(argument); } catch (...) { std::terminate(); }
    }
}
void __sl_cxx_finalize() noexcept {
    __sl_cxx_tls_finalize();
    for (;;) {
        if (pthread_mutex_lock(&mutex)) std::terminate();
        Destructor* p = process_destructors;
        if (p) process_destructors = p->next;
        if (pthread_mutex_unlock(&mutex)) std::terminate();
        if (!p) return;
        auto function = p->function;
        void* argument = p->argument;
        free(p);
        try { function(argument); } catch (...) { std::terminate(); }
    }
}
[[noreturn]] void __cxa_bad_typeid() { throw std::bad_typeid(); }
[[noreturn]] void __cxa_bad_cast() { throw std::bad_cast(); }
}
