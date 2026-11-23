#include <pthread.h>
#define FIXTURE "parity-musl"
#define TLS _Thread_local
#include "checks.h"

static void *worker(void *arg) {
    check_worker(*(int *)arg);
    return arg;
}

int main(void) {
    MARK("START");
    if (!check_file()) return 10;
    MARK("FILE PASS");
    if (local != 37) return 20;
    local = 73;
    int ids[] = {0, 1};
    pthread_t threads[2];
    for (int i = 0; i < 2; ++i)
        if (pthread_create(&threads[i], NULL, worker, &ids[i])) {
            __atomic_store_n(&release_workers, 1, __ATOMIC_RELEASE);
            if (i) pthread_join(threads[0], NULL);
            return 30;
        }
    int ok = rendezvous();
    for (int i = 0; i < 2; ++i) {
        void *result = NULL;
        if (pthread_join(threads[i], &result) || result != &ids[i])
            return 31;
    }
    if (!ok || local != 73 || completed[0] != 100 || completed[1] != 101)
        return 21;
    MARK("TLS PASS");
    MARK("JOIN PASS");
    return 0;
}
