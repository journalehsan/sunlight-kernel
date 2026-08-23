/* Minimal declarations required to compile unmodified sbase echo.c.
 * This is a portability header, not a modified application.
 */
#ifndef SBASE_UTIL_H
#define SBASE_UTIL_H

#include <stdio.h>

extern char *argv0;

void putword(FILE *fp, const char *s);
int fshut(FILE *fp, const char *fname);
void weprintf(const char *fmt, ...);

#endif
