/* SunlightOS native ABI contract probe. Requires argv[1] to be a disposable,
 * absent path on tmpfs. No Linux syscall or flag translation is used here. */
#include <stdint.h>
#include <string.h>
static intptr_t call3(uint64_t op, uint64_t a, uint64_t b, uint64_t c) {
    uint64_t result;
    __asm__ volatile ("syscall" : "=a"(result) : "a"(op), "D"(a), "S"(b), "d"(c)
                      : "rcx", "r11", "memory");
    return (intptr_t)result;
}
int main(int argc, char **argv) {
    if (argc != 2) return 10;
    /* open=40; RDWR=2, CREAT=0x40, EXCL=0x80, mode=0600. */
    intptr_t fd = call3(40, (uintptr_t)argv[1], 2 | 0x40 | 0x80, 0600);
    if (fd < 0) return 11;
    intptr_t duplicate = call3(45, (uint64_t)fd, 0, 0);
    int result = 0;
    if (duplicate < 0) { result = 12; goto cleanup; }
    if (call3(65, (uintptr_t)argv[1], 0, 0) != 0) { result = 13; goto cleanup; }
    /* Native ENOENT is -3, distinct from Linux's errno numbering. */
    intptr_t reopened = call3(40, (uintptr_t)argv[1], 0, 0);
    if (reopened != -3) {
        if (reopened >= 0) call3(41, (uint64_t)reopened, 0, 0);
        result = 14; goto cleanup;
    }
    if (call3(43, (uint64_t)fd, (uintptr_t)"retained", 8) != 8) { result = 15; goto cleanup; }
    if (call3(44, (uint64_t)duplicate, 0, 0) != 0) { result = 16; goto cleanup; }
    char bytes[8];
    if (call3(42, (uint64_t)duplicate, (uintptr_t)bytes, 8) != 8 || memcmp(bytes, "retained", 8)) {
        result = 17; goto cleanup;
    }
    if (call3(41, (uint64_t)fd, 0, 0) != 0) { fd = -1; result = 18; goto cleanup; }
    fd = -1;
    if (call3(44, (uint64_t)duplicate, 0, 0) != 0 ||
        call3(42, (uint64_t)duplicate, (uintptr_t)bytes, 8) != 8 || memcmp(bytes, "retained", 8)) result = 19;
cleanup:
    if (duplicate >= 0 && call3(41, (uint64_t)duplicate, 0, 0) != 0 && !result) result = 20;
    if (fd >= 0) call3(41, (uint64_t)fd, 0, 0);
    if (result) call3(65, (uintptr_t)argv[1], 0, 0);
    return result;
}
