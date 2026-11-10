/* Phase 6 guest contracts. The small IPC observer below is test-only and
 * mirrors ipc/src/lib.rs's native register ABI. It never generates randomness
 * or reads/logs random bytes, capabilities, seeds or keys. */
#include <sys/random.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static uint64_t syscall1(uint64_t number, uint64_t arg) {
    __asm__ volatile("syscall" : "+a"(number), "+D"(arg) : : "rcx", "r11", "memory");
    return number;
}

struct reply { uint64_t label, words[4]; };
static uint64_t ipc_once(uint64_t cap, uint64_t label, uint64_t word,
                         struct reply *reply) {
    uint64_t result = 1, counts = 1;
    register uint64_t w0 __asm__("r8") = word;
    register uint64_t w1 __asm__("r9") = 0;
    register uint64_t w2 __asm__("r10") = 0;
    register uint64_t w3 __asm__("r12") = 0;
    register uint64_t c0 __asm__("r13") = 0;
    register uint64_t c1 __asm__("r14") = 0;
    __asm__ volatile("syscall"
        : "+a"(result), "+D"(label), "+S"(cap), "+d"(counts),
          "+r"(w0), "+r"(w1), "+r"(w2), "+r"(w3), "+r"(c0), "+r"(c1)
        : : "rcx", "r11", "memory");
    reply->label = label;
    reply->words[0] = w0; reply->words[1] = w1;
    reply->words[2] = w2; reply->words[3] = w3;
    return result;
}

static int call(uint64_t cap, uint64_t label, uint64_t word, struct reply *reply) {
    /* Native MonotonicMs=86, IpcSetDeadline=8, IpcCall=1, Yield=21.
     * WouldBlock=3; the kernel's pending call preserves the armed deadline. */
    uint64_t deadline = syscall1(86, 0) + 2000;
    if (syscall1(8, deadline)) return -1;
    for (;;) {
        uint64_t result = ipc_once(cap, label, word, reply);
        if (result != 3) return result == 0 ? 0 : -1;
        syscall1(21, 0);
    }
}

struct stats { uint64_t requests, bytes; };
static int service_stats(struct stats *out) {
    struct reply reply;
    /* EndpointBind=11, init endpoint=0; InitMsg LOOKUP=2, GRANT=3.
     * Match ipc::name_to_u64's FNV-1a; RandMsg STATS=0x7202, REPLY=0x72ff. */
    uint64_t init = syscall1(11, 0);
    uint64_t name = UINT64_C(0xcbf29ce484222325);
    for (const unsigned char *p = (const unsigned char *)"rand"; *p; ++p)
        name = (name ^ *p) * UINT64_C(0x100000001b3);
    if (call(init, 2, name, &reply) || reply.label != 3) return -1;
    uint64_t rand_cap = reply.words[0];
    if (call(rand_cap, 0x7202, 0, &reply) || reply.label != 0x72ff || reply.words[0] != 1) return -1;
    out->requests = reply.words[1]; out->bytes = reply.words[2];
    return 0;
}

int main(int argc, char **argv) {
    if (argc != 2) return 60;
    struct stats before, after;
    if (service_stats(&before)) return 61;
    if (!strcmp(argv[1], "stats")) {
        return printf("rand service: requests=%llu bytes=%llu\n",
            (unsigned long long)before.requests, (unsigned long long)before.bytes) < 0 ? 62 : 0;
    }
    uint64_t requests = 0, bytes = 0;
    if (!strcmp(argv[1], "basic") || !strcmp(argv[1], "guards")) {
        const size_t lengths[] = {1, 16, 31, 32, 33, 65, 257};
        for (size_t i = 0; i < sizeof lengths / sizeof lengths[0]; ++i) {
            unsigned char storage[273];
            memset(storage, 0xab, sizeof storage);
            size_t n = !strcmp(argv[1], "basic") ? 16 : lengths[i];
            errno = 77;
            if (getrandom(storage + 7, n, 0) != (ssize_t)n || errno != 77) return 1;
            for (size_t j = 0; j < sizeof storage; ++j)
                if ((j < 7 || j >= 7 + n) && storage[j] != 0xab) return 2;
            requests += (n + 31) / 32; bytes += n;
            if (!strcmp(argv[1], "basic")) break;
        }
    } else if (!strcmp(argv[1], "zero")) {
        errno = 77;
        if (getrandom(NULL, 0, 0) || errno != 77) return 3;
        if (getrandom((void *)(uintptr_t)1, 0, 0) || errno != 77) return 4;
    } else if (!strcmp(argv[1], "errors")) {
        unsigned char b[16]; memset(b, 0xab, sizeof b);
        if (getrandom(NULL, 1, 0) != -1 || errno != EFAULT) return 5;
        if (getrandom(b, SIZE_MAX, 0) != -1 || errno != EINVAL) return 6;
        const unsigned int flags[] = {1, 2, 3, 4, 0x80000000U, ~0U};
        for (size_t i = 0; i < sizeof flags / sizeof flags[0]; ++i) {
            if (getrandom(b, sizeof b, flags[i]) != -1 || errno != EINVAL) return 7;
            if (getrandom(NULL, 0, flags[i]) != -1 || errno != EINVAL) return 8;
        }
        for (size_t i = 0; i < sizeof b; ++i) if (b[i] != 0xab) return 9;
    } else return 63;
    if (service_stats(&after)) return 64;
    /* Exact service accounting: empty/invalid calls consume no random output;
     * nonempty calls require the expected number of real 32-byte IPC replies. */
    if (after.requests - before.requests != requests || after.bytes - before.bytes != bytes) return 10;
    return 0;
}
