#include <locale.h>
#include <langinfo.h>
#include <string.h>
#include <errno.h>

int main(int argc, char **argv) {
    (void)argv;
    if (strcmp(setlocale(LC_ALL, NULL), "C")) return 1;
    if (strcmp(nl_langinfo(CODESET), "US-ASCII")) return 2;
    if (argc > 1) {
        if (!setlocale(LC_ALL, "") || strcmp(setlocale(LC_ALL, NULL), "en_US.UTF-8")) return 3;
        if (strcmp(nl_langinfo(CODESET), "UTF-8")) return 4;
        return 0;
    }
    errno = EINVAL;
    if (setlocale(-1, "C") || setlocale(7, NULL)) return 5;
    if (!setlocale(LC_ALL, "POSIX") || strcmp(setlocale(LC_ALL, NULL), "C")) return 6;
    if (!setlocale(LC_CTYPE, "C.UTF-8") || strcmp(nl_langinfo(CODESET), "UTF-8")) return 7;
    if (strcmp(setlocale(LC_NUMERIC, NULL), "C")) return 8;
    char saved[256];
    const char *mixed = setlocale(LC_ALL, NULL);
    memcpy(saved, mixed, strlen(mixed) + 1);
    if (strncmp(saved, "LC_CTYPE=C.UTF-8;LC_NUMERIC=C;", 30)) return 9;
    if (setlocale(LC_ALL, "de_DE.UTF-8") || strcmp(saved, setlocale(LC_ALL, NULL))) return 10;
    if (!setlocale(LC_ALL, saved) || strcmp(saved, setlocale(LC_ALL, NULL))) return 11;
    if (!setlocale(LC_ALL, setlocale(LC_ALL, NULL))) return 12;
    if (!setlocale(LC_ALL, "en_US.UTF-8")) return 13;
    if (!setlocale(LC_ALL, "") || strcmp(setlocale(LC_ALL, NULL), "C")) return 14;
    if (strcmp(nl_langinfo(CODESET), "US-ASCII") || errno != EINVAL) return 15;
    return 0;
}
