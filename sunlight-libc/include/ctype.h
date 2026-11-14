#ifndef SUNLIGHT_CTYPE_H
#define SUNLIGHT_CTYPE_H
/* Byte classification for the native C and UTF-8 locales: ASCII classes;
 * UTF-8 multibyte characters are decoded through wchar APIs, not ctype. */
#ifdef __cplusplus
extern "C" {
#endif
int isalnum(int);
int isalpha(int);
int isblank(int);
int iscntrl(int);
int isdigit(int);
int isgraph(int);
int islower(int);
int isprint(int);
int ispunct(int);
int isspace(int);
int isupper(int);
int isxdigit(int);
int tolower(int);
int toupper(int);
#ifdef __cplusplus
}
#endif
#endif
