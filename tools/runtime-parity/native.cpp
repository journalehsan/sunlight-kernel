#include <thread>
#define FIXTURE "parity-native"
#define TLS thread_local
#include "checks.h"

int main() {
    MARK("START");
    if (!check_file()) return 10;
    MARK("FILE PASS");
    if (local != 37) return 20;
    local = 73;
    std::thread first([] { check_worker(0); });
    std::thread second([] { check_worker(1); });
    bool ok = rendezvous();
    first.join();
    second.join();
    if (first.joinable() || second.joinable()) return 31;
    if (!ok || local != 73 || completed[0] != 100 || completed[1] != 101)
        return 21;
    MARK("TLS PASS");
    MARK("JOIN PASS");
    return 0;
}
