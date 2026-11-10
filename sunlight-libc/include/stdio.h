#ifndef SUNLIGHT_STDIO_H
#define SUNLIGHT_STDIO_H
#include <stddef.h>
#include <stdarg.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Opaque, locked streams. All streams are unbuffered in this foundation. */
typedef struct SunlightFile FILE;
#define EOF (-1)
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2
extern FILE *stdin;
extern FILE *stdout;
extern FILE *stderr;
FILE *fopen(const char *path, const char *mode);
/* Private read/write file on the native /tmp tmpfs, unlinked before return.
 * Uses exclusive creation and requires the native entropy service. No name
 * secrecy is promised. Failure returns NULL and sets errno. */
FILE *tmpfile(void);
int fclose(FILE *stream);
size_t fread(void *ptr, size_t size, size_t count, FILE *stream);
size_t fwrite(const void *ptr, size_t size, size_t count, FILE *stream);
int fflush(FILE *stream);
int fileno(FILE *stream);
int fgetc(FILE *stream);
int getc(FILE *stream);
int getchar(void);
int fputc(int ch, FILE *stream);
int putc(int ch, FILE *stream);
int putchar(int ch);
int fputs(const char *text, FILE *stream);
int puts(const char *text);
char *fgets(char *text, int size, FILE *stream);
int feof(FILE *stream);
int ferror(FILE *stream);
void clearerr(FILE *stream);
int fseek(FILE *stream, long offset, int whence);
long ftell(FILE *stream);
void rewind(FILE *stream);
/* Integer/string subset: c s d i u o x X p %, flags, width, precision,
 * hh/h/l/ll/j/z/t. Float, %n, wide and positional formats fail with ENOSYS;
 * preceding output may already have been written. No silent substitution. */
int printf(const char *format, ...);
int fprintf(FILE *stream, const char *format, ...);
int vprintf(const char *format, va_list args);
int vfprintf(FILE *stream, const char *format, va_list args);
#ifdef __cplusplus
}
#endif
#endif
