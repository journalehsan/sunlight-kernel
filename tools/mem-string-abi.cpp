#include <string.h>

int sunlight_libc_memory_string_cpp_abi_probe(const char *text) {
    return strchr(text, 'x') == strrchr(text, 'x');
}
