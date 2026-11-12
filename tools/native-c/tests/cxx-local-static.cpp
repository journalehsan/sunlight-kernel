#include <stdexcept>
#include <string>
#include <thread>
#include <time.h>
#include <unistd.h>
#include <string.h>

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

static void out(const char* text) {
    write(1, text, strlen(text));
}

static void sleep_ms(long milliseconds) {
    timespec request{milliseconds / 1000, (milliseconds % 1000) * 1000000};
    nanosleep(&request, nullptr);
}

static int seed() {
    return 7;
}

static int basic_constructed;
struct Basic {
    int value;
    std::string text;
    Basic() : value(seed() * 6), text("basic") { ++basic_constructed; }
    ~Basic() { out("basic:D\n"); }
};
static Basic& basic() {
    static Basic object;
    return object;
}

static int concurrent_arrived;
static int concurrent_constructed;
static int concurrent_errors;
static constexpr int kConcurrentThreads = 4;
struct Concurrent {
    int values[8];
    std::string text;
    Concurrent() : text("published") {
        while (__atomic_load_n(&concurrent_arrived, __ATOMIC_ACQUIRE) != kConcurrentThreads)
            sleep_ms(1);
        sleep_ms(100);
        for (int i = 0; i < 8; ++i) values[i] = 1000 + i;
        __atomic_add_fetch(&concurrent_constructed, 1, __ATOMIC_ACQ_REL);
    }
    ~Concurrent() { out("concurrent:D\n"); }
};
static Concurrent& concurrent() {
    static Concurrent object;
    return object;
}

static int retry_attempts;
static int retry_constructed;
static int retry_destroyed;
static int retry_arrived;
static int retry_errors;
static constexpr int kRetryThreads = 4;
struct Retry {
    std::string text;
    Retry() : text("retry") {
        int attempt = __atomic_add_fetch(&retry_attempts, 1, __ATOMIC_ACQ_REL);
        if (attempt == 1) {
            while (__atomic_load_n(&retry_arrived, __ATOMIC_ACQUIRE) != kRetryThreads)
                sleep_ms(1);
            sleep_ms(100);
            throw std::runtime_error("first attempt");
        }
        __atomic_add_fetch(&retry_constructed, 1, __ATOMIC_ACQ_REL);
    }
    ~Retry() {
        __atomic_add_fetch(&retry_destroyed, 1, __ATOMIC_RELAXED);
        out("retry:D\n");
    }
};
static Retry& retry() {
    static Retry object;
    return object;
}

static int nested_inner_constructed;
static int nested_outer_constructed;
struct Inner {
    int value;
    Inner() : value(seed()) { ++nested_inner_constructed; }
    ~Inner() { out("inner:D\n"); }
};
static Inner& inner();
struct Outer {
    int value;
    Outer() : value(inner().value + 1) { ++nested_outer_constructed; }
    ~Outer() { out("outer:D\n"); }
};
static Inner& inner() {
    static Inner object;
    return object;
}
static Outer& outer() {
    static Outer object;
    return object;
}

struct First {
    int value;
    First() : value(seed()) {}
    ~First() { out("first:D\n"); }
};
struct Second {
    int value;
    Second() : value(seed() + 10) {}
    ~Second() { out("second:D\n"); }
};
static First& first() {
    static First object;
    return object;
}
static Second& second() {
    static Second object;
    return object;
}

#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Winfinite-recursion"
static int& recursive() {
    static int value = recursive() + 1;
    return value;
}
#pragma clang diagnostic pop

static int run_basic() {
    Basic* first_address = &basic();
    Basic* second_address = &basic();
    if (first_address != second_address || first_address->value != 42 ||
        first_address->text != "basic" || basic_constructed != 1)
        return 1;
    out("basic:OK\n");
    return 0;
}

static int run_concurrent() {
    std::thread threads[kConcurrentThreads];
    for (int i = 0; i < kConcurrentThreads; ++i)
        threads[i] = std::thread([] {
            __atomic_add_fetch(&concurrent_arrived, 1, __ATOMIC_ACQ_REL);
            Concurrent& object = concurrent();
            if (object.text != "published")
                __atomic_add_fetch(&concurrent_errors, 1, __ATOMIC_RELAXED);
            for (int i = 0; i < 8; ++i)
                if (object.values[i] != 1000 + i)
                    __atomic_add_fetch(&concurrent_errors, 1, __ATOMIC_RELAXED);
        });
    for (int i = 0; i < kConcurrentThreads; ++i)
        threads[i].join();
    Concurrent& object = concurrent();
    if (concurrent_constructed != 1 || concurrent_errors != 0 || object.text != "published")
        return 2;
    for (int i = 0; i < 8; ++i)
        if (object.values[i] != 1000 + i) return 3;
    out("concurrent:OK\n");
    return 0;
}

static int run_retry() {
    std::thread threads[kRetryThreads];
    for (int i = 0; i < kRetryThreads; ++i)
        threads[i] = std::thread([] {
            __atomic_add_fetch(&retry_arrived, 1, __ATOMIC_ACQ_REL);
            try {
                Retry& object = retry();
                if (object.text != "retry")
                    __atomic_add_fetch(&retry_errors, 1, __ATOMIC_RELAXED);
            } catch (const std::runtime_error&) {
            }
        });
    for (int i = 0; i < kRetryThreads; ++i)
        threads[i].join();
    Retry& object = retry();
    if (retry_attempts != 2 || retry_constructed != 1 || retry_destroyed != 0 ||
        retry_errors != 0 ||
        object.text != "retry")
        return 4;
    out("retry:OK\n");
    return 0;
}

static int run_nested() {
    if (inner().value != 7 || outer().value != 8 ||
        nested_inner_constructed != 1 || nested_outer_constructed != 1)
        return 5;
    out("nested:OK\n");
    return 0;
}

static int run_order() {
    if (first().value != 7 || second().value != 17)
        return 6;
    out("order:OK\n");
    return 0;
}

static int run_recursive() {
    out("recursive:armed\n");
    (void)recursive();
    return 7;
}

struct FinalCheck {
    ~FinalCheck() {
        if (live) sl_std_exit(77);
        if (retry_destroyed > 1) sl_std_exit(78);
    }
} final_check;

int main(int argc, char** argv) {
    if (argc < 2) return 90;
    if (!strcmp(argv[1], "basic")) return run_basic();
    if (!strcmp(argv[1], "concurrent")) return run_concurrent();
    if (!strcmp(argv[1], "retry")) return run_retry();
    if (!strcmp(argv[1], "nested")) return run_nested();
    if (!strcmp(argv[1], "order")) return run_order();
    if (!strcmp(argv[1], "recursive")) return run_recursive();
    return 91;
}
