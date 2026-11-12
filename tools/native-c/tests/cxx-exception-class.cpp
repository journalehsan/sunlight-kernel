#include <stdexcept>
#include <typeinfo>
static __SIZE_TYPE__ length(const char* s) noexcept { const char* p=s; while (*p) ++p; return p-s; }
static int compare(const char* a, const char* b) noexcept { while (*a && *a == *b) { ++a; ++b; } return *a-*b; }
extern "C" void* malloc(__SIZE_TYPE__) noexcept;
extern "C" void free(void*) noexcept;
extern "C" void* __real_malloc(__SIZE_TYPE__) noexcept;
extern "C" void __real_free(void*) noexcept;
static unsigned allocations, releases;
extern "C" void* __wrap_malloc(__SIZE_TYPE__ n) noexcept {
    void* p = __real_malloc(n);
    if (p) ++allocations;
    return p;
}
extern "C" void __wrap_free(void* p) noexcept {
    if (p) ++releases;
    __real_free(p);
}
void* operator new(__SIZE_TYPE__, void* p) noexcept { return p; }
static_assert(noexcept(std::runtime_error(*(std::runtime_error*)nullptr)), "copy must not throw");
static_assert(noexcept(*(std::runtime_error*)nullptr = *(std::runtime_error*)nullptr), "assignment must not throw");
static_assert(noexcept(((std::exception*)nullptr)->what()), "what must not throw");
static int verify() noexcept {
    char message[8193];
    for (unsigned i = 0; i < sizeof(message)-1; ++i) message[i] = 'a' + i % 26;
    message[sizeof(message)-1] = 0;
    std::runtime_error source(message);
    std::runtime_error copy(source);
    std::runtime_error assigned("old");
    assigned = source;
    std::runtime_error* alias = &assigned;
    assigned = *alias;
    message[0] = '!';
    if (length(source.what()) != 8192 || source.what()[0] != 'a' ||
        compare(source.what(), copy.what()) || compare(copy.what(), assigned.what())) return 1;
    const char* shared = copy.what();
    source = std::runtime_error("replacement");
    if (compare(shared, assigned.what()) || shared[0] != 'a') return 2;
    void* storage = malloc(sizeof(std::runtime_error));
    if (!storage) return 3;
    std::exception* base = new(storage) std::runtime_error(copy);
    if (compare(base->what(), copy.what())) return 4;
    if (!dynamic_cast<std::runtime_error*>(base)) return 5;
    delete base; // real virtual deleting destructor and paired native free
    std::exception plain;
    if (compare(plain.what(), "std::exception") || dynamic_cast<std::runtime_error*>(&plain)) return 6;
    std::string binary("a\0b", 3);
    std::runtime_error embedded(binary);
    binary = "changed";
    if (embedded.what()[0] != 'a' || embedded.what()[1] || embedded.what()[2] != 'b' || embedded.what()[3]) return 8;
    return 0;
}
int main() {
    for (unsigned i = 0; i < 2000; ++i) {
        int result = verify();
        if (result) return result;
        if (allocations != releases) return 7;
    }
    return 0;
}
