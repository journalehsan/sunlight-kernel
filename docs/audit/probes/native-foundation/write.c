#include <sunlight_runtime.h>

int main(void) {
    static const unsigned char message[] = "[NATIVE-C] write OK\n";
    /* Canonical stderr path is available before terminal/session routing. */
    return sl_std_write(2, message, sizeof(message) - 1) == (intptr_t)(sizeof(message) - 1)
        ? 0 : 12;
}
