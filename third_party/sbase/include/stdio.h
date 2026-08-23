#ifndef HELIOS_MINILIBC_STDIO_H
#define HELIOS_MINILIBC_STDIO_H

typedef struct HeliosFile FILE;

extern FILE *stdout;
extern FILE *stderr;

int fputc(int c, FILE *fp);
int putchar(int c);
int fputs(const char *s, FILE *fp);
int fflush(FILE *fp);
int fclose(FILE *fp);
int ferror(FILE *fp);
int vfprintf(FILE *fp, const char *fmt, __builtin_va_list ap);

#endif
