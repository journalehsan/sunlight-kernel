#include <sunlight_runtime.h>

int main(void) {
    /* A guest runner must observe status 23, not classify it as probe failure. */
    sl_std_exit(23);
}
