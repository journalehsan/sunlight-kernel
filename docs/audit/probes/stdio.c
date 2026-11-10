#define _GNU_SOURCE
#include <stdio.h>
#include <errno.h>
#include <string.h>
/* Retains the historical tmpfile/fputs/rewind/fgetc round trip, then checks
 * independent streams, positioning and indicators using the same FILE API. */
int main(void) {
    FILE *f = tmpfile();
    if (!f) return 1;
    int result = 0;
    FILE *other = NULL;
    if (fputs("x", f) < 0) { result = 2; goto done; }
    rewind(f);
    if (fgetc(f) != 'x') { result = 3; goto done; }
    if (fgetc(f) != EOF || !feof(f) || ferror(f)) { result = 4; goto done; }
    other = tmpfile();
    if (!other) { result = 5; goto done; }
    if (fileno(f) == fileno(other) || fwrite("two", 1, 3, other) != 3) { result = 6; goto done; }
    if (fseek(f, 0, SEEK_END) || feof(f) || ftell(f) != 1 || fputs("yz", f) < 0) { result = 7; goto done; }
    rewind(f);
    char bytes[4] = {0};
    if (fread(bytes, 1, 4, f) != 3 || memcmp(bytes, "xyz", 3) || !feof(f)) { result = 8; goto done; }
    if (fseek(f, 0, 99) != -1 || !ferror(f) || errno != EINVAL) { result = 9; goto done; }
    rewind(f);
    if (feof(f) || ferror(f) || fgetc(f) != 'x') { result = 10; goto done; }
    rewind(other);
    if (fread(bytes, 1, 3, other) != 3 || memcmp(bytes, "two", 3)) result = 11;
done:
    if (other && fclose(other) && !result) result = 12;
    if (fclose(f) && !result) result = 13;
    return result;
}
