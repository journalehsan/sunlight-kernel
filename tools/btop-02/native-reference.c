#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>

struct prefix {
    uint64_t magic;
    uint32_t version, sequence;
    uint64_t uptime, total_kb, used_kb;
};

int main(void) {
    uintptr_t raw = 95; /* SYS_MAP_TELEMETRY on the native ABI */
    __asm__ volatile("syscall" : "+a"(raw) :: "rcx", "r11", "memory");
    const volatile struct prefix *p = (const volatile struct prefix *)raw;
    if (!raw || raw == UINTPTR_MAX || p->magic != UINT64_C(0x53554e4c54494d45)) return 1;
    for (unsigned i = 0; i < 100000; ++i) {
        uint32_t before = __atomic_load_n(&p->sequence, __ATOMIC_ACQUIRE);
        if (!before || before & 1) continue;
        uint64_t total = p->total_kb, used = p->used_kb, uptime = p->uptime;
        __atomic_thread_fence(__ATOMIC_ACQUIRE);
        if (before != __atomic_load_n(&p->sequence, __ATOMIC_RELAXED)) continue;
        if (!total || used > total || p->version < 3 || p->version > 5) return 2;
        printf("NATIVE-TELEMETRY version=%" PRIu32 " free=%" PRIu64
               " total=%" PRIu64 " uptime=%" PRIu64 "\n", p->version, total - used, total, uptime);
        return 0;
    }
    return 3;
}
