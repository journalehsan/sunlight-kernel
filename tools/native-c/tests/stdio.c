/* Native: pass a disposable writable pathname as argv[1], and pipe Q\n to
 * descriptor 0. Host adapter execution is a separate acceptance layer. */
#include <stdio.h>
#include <errno.h>
#include <string.h>

static int print_list(FILE *file, const char *format, ...) {
    va_list args;
    va_start(args, format);
    int result = vfprintf(file, format, args);
    va_end(args);
    return result;
}
static int print_stdout(const char *format, ...) {
    va_list args;
    va_start(args, format);
    int result = vprintf(format, args);
    va_end(args);
    return result;
}
int main(int argc, char **argv) {
    if (argc != 2) return 10;
    if (fileno(stdin) != 0 || fileno(stdout) != 1 || fileno(stderr) != 2) return 11;
    if (getchar() != 'Q' || getc(stdin) != '\n') return 12;
    if (printf("stdio: %s %d %u %#x %ld %zu %%\n", "native", -42, 42u,
               42u, -1234567890L, (size_t)7) != 42) return 13;
    if (puts("stdio: puts") < 0 || putchar('!') != '!' || putchar('\n') != '\n') return 14;
    if (print_stdout("stdio: vprintf %c\n", 'V') != 17) return 15;
    if (fprintf(stderr, "stdio: stderr %04d\n", 7) != 19) return 16;
    if (fflush(stdout) || fflush(stderr) || fflush(NULL)) return 17;

    FILE *file = fopen(argv[1], "w+b");
    if (!file) return 20;
    if (fwrite("abc", 1, 3, file) != 3 || fputc('\n', file) != '\n') return 21;
    if (print_list(file, "%*.*s:%lld:%hhu\n", -4, 2, "xyz", -99LL, 255) != 13) return 22;
    if (ftell(file) != 17 || fflush(file) || fseek(file, 0, SEEK_SET)) return 23;
    char line[32];
    if (fgets(line, sizeof line, file) != line || strcmp(line, "abc\n")) return 24;
    if (fgets(line, sizeof line, file) != line || strcmp(line, "xy  :-99:255\n")) return 25;
    if (fgetc(file) != EOF || !feof(file) || ferror(file)) return 26;
    rewind(file);
    if (feof(file) || ferror(file) || getc(file) != 'a') return 27;
    if (fclose(file)) return 28;

    file = fopen(argv[1], "ab");
    if (!file || fseek(file, 0, SEEK_SET) || fputs("tail", file) < 0 || fclose(file)) return 30;
    file = fopen(argv[1], "rb");
    if (!file) return 31;
    char bytes[32] = {0};
    if (fread(bytes, 2, 16, file) != 10 || !feof(file) || ferror(file)) return 32;
    if (strcmp(bytes, "abc\nxy  :-99:255\ntail")) return 33;
    if (fputc('x', file) != EOF || !ferror(file) || errno != EBADF) return 34;
    clearerr(file);
    if (feof(file) || ferror(file) || fclose(file)) return 35;
    file = tmpfile();
    if (!file || fputs("temporary", file) < 0) return 36;
    rewind(file);
    if (fgetc(file) != 't' || fclose(file)) return 37;
    if (puts("stdio: file PASS; tmpfile PASS") < 0) return 38;
    return 0;
}
