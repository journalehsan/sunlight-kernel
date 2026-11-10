#include <sunlight_runtime.h>
int main(void) {
    static const unsigned char message[] = "[NATIVE-C] stdout OK\n";
    return sl_std_write(1, message, sizeof(message) - 1) == (intptr_t)(sizeof(message) - 1) ? 0 : 41;
}
