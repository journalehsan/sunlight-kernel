#include <string.h>

/*
 * Compile/link-only C ABI probe. The host Rust tests execute the behavioral
 * cases; this verifies that a C11 translation unit sees the same prototypes
 * and that each referenced symbol is resolved by the freestanding object.
 */
int sunlight_libc_memory_string_abi_probe(void) {
    unsigned char source[] = {0x80, 'b', 'c', 0};
    unsigned char destination[4] = {0};
    char text[] = "abca";

    if (memcpy(destination, source, 4) != destination) return 1;
    if (memmove(destination + 1, destination, 3) != destination + 1) return 2;
    if (memset(destination, 0x1ff, 1) != destination) return 3;
    if (memcmp(source, source, 4) != 0) return 4;
    if (memchr(source, 0x80, 4) != source) return 5;
    if (strlen(text) != 4 || strnlen(text, 2) != 2) return 6;
    if (strcmp(text, "abca") != 0 || strncmp(text, "abcz", 3) != 0) return 7;
    if (strchr(text, 'a') != text || strrchr(text, 'a') != text + 3) return 8;
    return 0;
}
