#include <string>
#include <stdlib.h>
#include <unistd.h>
static bool fail;
extern "C" void* __real_malloc(size_t);
extern "C" void* __wrap_malloc(size_t n) { return fail ? nullptr : __real_malloc(n); }
static void callback() { std::cout << "BAD callback\n"; }
int main(int argc, char** argv) {
    if (argc != 2 || atexit(callback)) return 1;
    std::string s("live");
    std::cout << "armed\n";
    if (argv[1][0] == 'o') s.append("x", std::string::max_size());
    else if (argv[1][0] == 'n') s.append(nullptr, 1);
    else { fail = true; s += " allocation must fail"; }
    std::cout << "BAD survived\n";
    return 2;
}
