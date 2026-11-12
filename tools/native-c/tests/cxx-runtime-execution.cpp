#include <stdexcept>
#include <thread>
#include <new>
#include <typeinfo>
#include <cstdlib>
#include <unistd.h>
extern "C" void* __real_malloc(__SIZE_TYPE__) noexcept;
extern "C" void __real_free(void*) noexcept;
static long live;
static int fail_alloc;
static int fail_thread;
extern "C" int __real_pthread_create(pthread_t*, const pthread_attr_t*, void* (*)(void*), void*);
extern "C" int __wrap_pthread_create(pthread_t* t, const pthread_attr_t* a, void* (*f)(void*), void* p) {
    if (fail_thread) { fail_thread = 0; return 11; }
    return __real_pthread_create(t, a, f, p);
}
extern "C" void* __wrap_malloc(__SIZE_TYPE__ n) noexcept {
    int policy = __atomic_load_n(&fail_alloc, __ATOMIC_RELAXED);
    if (policy && (policy == 2 || __atomic_exchange_n(&fail_alloc, 0, __ATOMIC_RELAXED))) return nullptr;
    void* p = __real_malloc(n);
    if (p) __atomic_add_fetch(&live, 1, __ATOMIC_RELAXED);
    return p;
}
extern "C" void __wrap_free(void* p) noexcept {
    if (p) __atomic_sub_fetch(&live, 1, __ATOMIC_RELAXED);
    __real_free(p);
}
struct Token { static int living; Token() { ++living; } Token(const Token&) { ++living; } ~Token() { --living; } };
int Token::living;
struct Cleanup { int* count; ~Cleanup() { ++*count; } };
struct Base { virtual ~Base() = default; };
struct Derived : Base {};
static int tls_done;
struct Local { int value = 0; ~Local() { __atomic_add_fetch(&tls_done, 1, __ATOMIC_RELAXED); } };
thread_local Local local;
static int ready;
static int errors;
static void worker(int id) {
    local.value = id;
    try {
        try { throw std::runtime_error(id == 1 ? "one" : "two"); }
        catch (const std::exception& e) {
            __atomic_add_fetch(&ready, 1, __ATOMIC_SEQ_CST);
            while (__atomic_load_n(&ready, __ATOMIC_SEQ_CST) != 2) {}
            if (e.what()[0] != (id == 1 ? 'o' : 't')) __atomic_add_fetch(&errors, 1, __ATOMIC_RELAXED);
            throw;
        }
    } catch (const std::runtime_error& e) {
        if (local.value != id || e.what()[0] != (id == 1 ? 'o' : 't')) __atomic_add_fetch(&errors, 1, __ATOMIC_RELAXED);
    }
}
static int callable_living;
struct Callable {
    Callable() { ++callable_living; }
    Callable(const Callable&) = delete;
    Callable(Callable&&) noexcept { ++callable_living; }
    ~Callable() { --callable_living; }
    void operator()() { local.value = 37; }
};
static void forbidden() { write(1, "callback\n", 9); }
int main(int argc, char** argv) {
    if (argc > 1) {
        std::atexit(forbidden);
        write(1, "armed\n", 6);
        switch (argv[1][0]) {
        case 'u': throw std::runtime_error("uncaught");
        case 'd': { std::thread t([] {}); return 71; }
        case 'w': { std::thread t([] { throw std::runtime_error("worker"); }); t.join(); return 72; }
        case 's': __atomic_store_n(&fail_alloc, 2, __ATOMIC_RELAXED); throw Token();
        }
        return 73;
    }
    long baseline = live;
    for (unsigned i=0; i<1500; ++i) {
        int cleaned = 0;
        try { Cleanup c{&cleaned}; throw std::runtime_error("typed"); }
        catch (const std::exception& e) { if (e.what()[0] != 't' || cleaned != 1) return 1; }
        try {
            try { throw Token(); }
            catch (...) { if (Token::living != 1) return 2; throw; }
        } catch (const Token&) { if (Token::living != 1) return 3; }
        if (Token::living || live != baseline) return 4;
    }
    try {
        __atomic_store_n(&fail_alloc, 1, __ATOMIC_RELAXED);
        auto* p = new Derived;
        delete p;
        return 5;
    } catch (const std::bad_alloc& e) { if (e.what()[0] != 's') return 6; }
    try { throw 42; } catch (int value) { if (value != 42) return 19; }
    Derived derived;
    Base* base = &derived;
    if (dynamic_cast<Derived*>(base) != &derived || typeid(*base) != typeid(Derived)) return 7;
    Base* null = nullptr;
    try { (void)typeid(*null); return 8; } catch (const std::bad_typeid&) {}
    fail_thread = 1;
    try { std::thread failed(Callable{}); return 17; }
    catch (const std::runtime_error&) {}
    if (callable_living || live != baseline) return 18;
    std::thread empty;
    if (empty.joinable()) return 9;
    try { empty.join(); return 10; } catch (const std::runtime_error&) {}
    {
        std::thread first([] { worker(1); });
        std::thread second([] { worker(2); });
        std::thread moved(static_cast<std::thread&&>(first));
        if (first.joinable() || !moved.joinable()) return 11;
        empty = static_cast<std::thread&&>(second);
        if (second.joinable() || !empty.joinable()) return 12;
        moved.join(); empty.join();
        if (moved.joinable() || empty.joinable()) return 13;
    }
    if (errors || tls_done != 2) return 14;
    for (int i=0; i<40; ++i) {
        Callable callable;
        std::thread t(static_cast<Callable&&>(callable));
        t.join();
        if (callable_living != 1) return 15;
    }
    if (callable_living || tls_done != 42 || live != baseline) return 16;
    return 0;
}
