#ifndef SUNLIGHT_LOCALE_H
#define SUNLIGHT_LOCALE_H
#include <stddef.h>
#define LC_CTYPE 0
#define LC_NUMERIC 1
#define LC_TIME 2
#define LC_COLLATE 3
#define LC_MONETARY 4
#define LC_MESSAGES 5
#define LC_ALL 6
#ifdef __cplusplus
extern "C" {
#endif
/* All installed names use C numeric and monetary conventions; en_US.UTF-8
 * currently provides UTF-8 character encoding, not US monetary formatting.
 * Returned metadata has process lifetime and must not be modified. */
struct lconv {
    char *decimal_point, *thousands_sep, *grouping;
    char *int_curr_symbol, *currency_symbol, *mon_decimal_point;
    char *mon_thousands_sep, *mon_grouping, *positive_sign, *negative_sign;
    char int_frac_digits, frac_digits, p_cs_precedes, p_sep_by_space;
    char n_cs_precedes, n_sep_by_space, p_sign_posn, n_sign_posn;
    char int_p_cs_precedes, int_p_sep_by_space, int_n_cs_precedes;
    char int_n_sep_by_space, int_p_sign_posn, int_n_sign_posn;
};
struct lconv *localeconv(void);
char *setlocale(int category, const char *locale);
#ifdef __cplusplus
}
#endif
#endif
