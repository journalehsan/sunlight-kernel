#define _GNU_SOURCE
#include <pthread.h>
#include <signal.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/syscall.h>
#include <ucontext.h>
#include <unistd.h>

#define BIT(sig) (UINT64_C(1) << ((sig)-1))
#define RCX UINT64_C(0x123456789abcdef0)
#define R11 UINT64_C(0xfedcba9876543210)
#define RESTORER UINT64_C(0x04000000)

_Atomic int ready, caught;
uint64_t spin_rsp, resumed_rsp, resumed_rcx, resumed_r11, resumed_flags;
uint64_t resumed_xmm, resumed_redzone;
extern char spin_begin[], spin_end[];
extern void sig01_spin(void);

static int target_tid, sender_tid;
static uint64_t inside_mask, sender_mask, saved_mask, saved_rip, saved_rsp;
static uint64_t saved_rcx, saved_r11, saved_flags, saved_cs, uc_flags;
static unsigned fp_magic;
static int handler_tid, info_pid, info_code, info_signo;
static const char *mode = "";

static uint64_t mask(void)
{
    uint64_t result = ~UINT64_C(0);
    if (syscall(SYS_rt_sigprocmask, SIG_SETMASK, 0, &result, sizeof result))
        _exit(10);
    return result;
}

static void handler(int sig, siginfo_t *info, void *context)
{
    ucontext_t *uc = context;
    greg_t *g = uc->uc_mcontext.gregs;
    handler_tid = (int)syscall(SYS_gettid);
    inside_mask = mask();
    memcpy(&saved_mask, &uc->uc_sigmask, sizeof saved_mask);
    saved_rip = g[REG_RIP];
    saved_rsp = g[REG_RSP];
    saved_rcx = g[REG_RCX];
    saved_r11 = g[REG_R11];
    saved_flags = g[REG_EFL];
    saved_cs = g[REG_CSGSFS];
    uc_flags = uc->uc_flags;
    if (!uc->uc_mcontext.fpregs)
        _exit(11);
    unsigned char *fp = (void *)uc->uc_mcontext.fpregs;
    memcpy(&fp_magic, fp + 464, sizeof fp_magic);
    info_signo = info->si_signo;
    info_pid = info->si_pid;
    info_code = info->si_code;
    if (sig != SIGUSR1 || handler_tid != target_tid)
        _exit(12);

    if (mode[0]) {
        if (!strcmp(mode, "selector"))
            g[REG_CSGSFS] ^= 8;
        else if (!strcmp(mode, "flags"))
            g[REG_EFL] |= 3 << 12;
        else if (!strcmp(mode, "rip"))
            g[REG_RIP] = 1;
        else if (!strcmp(mode, "noncanon"))
            g[REG_RIP] = UINT64_C(0x0000800000000000);
        else if (!strcmp(mode, "rsp"))
            g[REG_RSP] = 1;
        else if (!strcmp(mode, "fpptr"))
            uc->uc_mcontext.fpregs = NULL;
        else if (!strcmp(mode, "fpstate")) {
            const uint32_t xstate_magic = 0x46505853;
            memcpy(fp + 464, &xstate_magic, sizeof xstate_magic);
        }
        else if (!strcmp(mode, "altstack"))
            uc->uc_stack.ss_flags = 0;
        else
            _exit(13);
        if (write(1, "SIG01 bad-frame armed\n", 22) != 22)
            _exit(14);
    }
    __asm__ volatile ("pxor %%xmm15, %%xmm15" ::: "xmm15");
    atomic_fetch_add_explicit(&caught, 1, memory_order_release);
}

static void *sender(void *arg)
{
    (void)arg;
    sender_tid = (int)syscall(SYS_gettid);
    while (!atomic_load_explicit(&ready, memory_order_acquire))
        __asm__ volatile ("pause");
    if (syscall(SYS_tkill, target_tid, SIGUSR1))
        _exit(15);
    sender_mask = mask();
    return NULL;
}

int main(int argc, char **argv)
{
    if (argc == 2)
        mode = argv[1];
    if (!strcmp(mode, "noactive")) {
        if (write(1, "SIG01 no-active armed\n", 22) != 22)
            return 14;
        syscall(SYS_rt_sigreturn);
        return 16;
    }
    target_tid = (int)syscall(SYS_gettid);
    sigset_t orig;
    sigemptyset(&orig);
    sigaddset(&orig, SIGUSR2);
    if (pthread_sigmask(SIG_SETMASK, &orig, NULL))
        return 17;

    struct sigaction act = { .sa_sigaction = handler, .sa_flags = SA_SIGINFO | SA_RESTART };
    sigemptyset(&act.sa_mask);
    sigaddset(&act.sa_mask, SIGINT);
    if (sigaction(SIGUSR1, &act, NULL))
        return 18;
    struct { uint64_t handler, flags, restorer, mask; } wire = {0};
    if (syscall(SYS_rt_sigaction, SIGUSR1, NULL, &wire, 8) ||
        wire.handler != (uintptr_t)handler || !(wire.flags & RESTORER) ||
        !wire.restorer || wire.mask != BIT(SIGINT))
        return 19;

    pthread_t thread;
    if (pthread_create(&thread, NULL, sender, NULL))
        return 20;
    sig01_spin();
    if (pthread_join(thread, NULL))
        return 21;
    uint64_t restored = mask();
    int ok = !mode[0] && caught == 1 && handler_tid == target_tid &&
        handler_tid != sender_tid && sender_mask == BIT(SIGUSR2) &&
        saved_mask == BIT(SIGUSR2) && restored == saved_mask &&
        inside_mask == (BIT(SIGUSR2) | BIT(SIGUSR1) | BIT(SIGINT)) &&
        info_signo == SIGUSR1 && info_pid == getpid() && info_code == SI_TKILL &&
        saved_rip >= (uintptr_t)spin_begin && saved_rip < (uintptr_t)spin_end &&
        saved_rcx == RCX && saved_r11 == R11 && resumed_rcx == saved_rcx &&
        resumed_r11 == saved_r11 && (saved_flags & ~(UINT64_C(1) << 16)) == resumed_flags && (resumed_flags & 0x401) == 0x401 &&
        resumed_xmm == RCX && resumed_redzone == RCX &&
        spin_rsp == saved_rsp && resumed_rsp == spin_rsp;

    printf("SIG01 %s count=%d target=%d handler=%d sender=%d masks=%llx/%llx/%llx/%llx "
           "action=%llx restorer=%llx uc=%llx selectors=%llx fp=%x\n",
           ok ? "PASS" : "FAIL", caught, target_tid, handler_tid, sender_tid,
           (unsigned long long)saved_mask, (unsigned long long)inside_mask,
           (unsigned long long)restored, (unsigned long long)sender_mask,
           (unsigned long long)wire.flags, (unsigned long long)wire.restorer,
           (unsigned long long)uc_flags, (unsigned long long)saved_cs, fp_magic);
    printf("SIG01 context rip=%llx rcx=%llx/%llx r11=%llx/%llx flags=%llx/%llx "
           "rsp=%llx/%llx xmm=%llx redzone=%llx\n",
           (unsigned long long)saved_rip, (unsigned long long)saved_rcx,
           (unsigned long long)resumed_rcx, (unsigned long long)saved_r11,
           (unsigned long long)resumed_r11, (unsigned long long)saved_flags,
           (unsigned long long)resumed_flags, (unsigned long long)saved_rsp,
           (unsigned long long)resumed_rsp, (unsigned long long)resumed_xmm,
           (unsigned long long)resumed_redzone);
    return ok ? 0 : 22;
}
