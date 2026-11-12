#include <exception>
extern "C" int __real_main(int, char**);
extern "C" int atexit(void (*)()) noexcept;
extern "C" void __sl_cxx_finalize() noexcept;
extern "C" void __sl_cxx_tls_finalize() noexcept;
extern "C" void (*__sl_init_array_start[])();
extern "C" void (*__sl_init_array_end[])();
extern "C" int __wrap_main(int argc, char** argv) noexcept {
    try {
        if (atexit(__sl_cxx_finalize)) std::terminate();
        for (auto* p = __sl_init_array_start; p != __sl_init_array_end; ++p) (*p)();
        int result = __real_main(argc, argv);
        __sl_cxx_tls_finalize();
        return result;
    }
    catch (...) { std::terminate(); }
}
