#include <thread>
#include <string>
#include <iostream>
#include <cstdlib>
#include <unistd.h>
extern "C" void* __real_malloc(__SIZE_TYPE__) noexcept;
extern "C" void __real_free(void*) noexcept;
extern "C" [[noreturn]] void sl_std_exit(int) noexcept;
static long live;
extern "C" void* __wrap_malloc(__SIZE_TYPE__ n) noexcept {
    void* p = __real_malloc(n);
    if (p) __atomic_add_fetch(&live, 1, __ATOMIC_RELAXED);
    return p;
}
extern "C" void __wrap_free(void* p) noexcept {
    if (p) __atomic_sub_fetch(&live, 1, __ATOMIC_RELAXED);
    __real_free(p);
}
struct GlobalWatcher {
    GlobalWatcher() { std::cout << "global-init\n"; }
    ~GlobalWatcher() {
        if (live) sl_std_exit(77);
        std::cout << "global-done\n";
    }
} watcher;
std::string global("global");
struct TlsWatcher { ~TlsWatcher() { std::cout << "TLS-done\n"; } };
thread_local TlsWatcher tls_watcher;
thread_local std::string local("TLS");
static void done() { std::cout << "atexit\n"; }
int main() {
    if (std::atexit(done)) return 1;
    std::thread worker([] { (void)tls_watcher; std::cout << "worker:" << local << '\n'; });
    worker.join();
    (void)tls_watcher;
    std::cout << "main:" << global << ':' << local << '\n';
    return 0;
}
