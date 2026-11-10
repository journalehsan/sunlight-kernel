/* Leave real anonymous VFS files and heap allocations for process-death cleanup. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(void) {
    FILE *a = tmpfile(), *b = tmpfile();
    if (!a || !b) return 51;
    char *page = malloc(8192);
    if (!page) return 52;
    memset(page, 'L', 8192);
    if (fwrite(page, 1, 8192, a) != 8192 || fputs("unclosed", b) < 0) return 53;
    /* No fclose/free: the actual scheduler reaper must drain both descriptors
     * and return all private, page-table and anonymous file-data frames. */
    return 0;
}
