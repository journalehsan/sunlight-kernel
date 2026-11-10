#include <dlfcn.h>
#include <stdlib.h>
#include <stdint.h>
int main(void) {
    if (dlopen("lib.so", RTLD_NOW) || dlopen(0, 0)) return 1;
    if (dlsym((void *)(uintptr_t)1, "malloc") || dlclose((void *)(uintptr_t)1) != -1) return 2;
    void *h = dlopen(0, RTLD_NOW);
    if (!h) return 3;
    void *(*allocate)(size_t) = (void *(*)(size_t))dlsym(h, "malloc");
    void (*release)(void *) = (void (*)(void *))dlsym(h, "free");
    if (allocate != malloc || release != free) return 4;
    if (dlsym(h, "missing") || dlsym(h, 0) || dlsym(h, "")) return 5;
    unsigned char *p = (unsigned char *)allocate(64);
    if (!p) return 6;
    for (int i = 0; i < 64; i++) p[i] = (unsigned char)i;
    release(p);
    void *h2 = dlopen(0, RTLD_NOW);
    if (!h2 || dlclose(h) || !dlsym(h2, "calloc") || dlclose(h2)) return 7;
    if (dlsym(h, "malloc") || dlclose(h) != -1) return 8;
    return 0;
}
