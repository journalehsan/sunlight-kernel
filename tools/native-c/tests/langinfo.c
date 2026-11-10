#include <langinfo.h>
#include <string.h>
#include <errno.h>
#include <limits.h>

int main(void) {
    errno = EINVAL;
    char *codeset = nl_langinfo(CODESET);
    if (!codeset || strcmp(codeset, "US-ASCII") || errno != EINVAL) return 1;
    const nl_item invalid[] = {INT_MIN, -1, 0, CODESET + 1, INT_MAX};
    for (unsigned i = 0; i < sizeof(invalid) / sizeof(invalid[0]); ++i) {
        char *empty = nl_langinfo(invalid[i]);
        if (!empty || *empty || errno != EINVAL) return 2;
    }
    if (codeset != nl_langinfo(CODESET) || strcmp(codeset, "US-ASCII")) return 3;
    return 0;
}
