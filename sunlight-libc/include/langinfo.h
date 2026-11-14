#ifndef SUNLIGHT_LANGINFO_H
#define SUNLIGHT_LANGINFO_H

/* Native locale metadata: CODESET follows LC_CTYPE; calendar, numeric and
 * monetary metadata retain the installed C baseline for all accepted names.
 * Format strings describe C conventions; they do not advertise strftime or
 * timezone support. Storage has process lifetime and must not be modified or
 * freed. Unsupported items return an empty string without changing errno.
 * nl_item values are native ABI constants, not a foreign libc's encoding. */
typedef int nl_item;
#define CODESET 1
/* Keep the original reserved values (including 2) unsupported. */
#define DAY_1 256
#define DAY_2 257
#define DAY_3 258
#define DAY_4 259
#define DAY_5 260
#define DAY_6 261
#define DAY_7 262
#define ABDAY_1 263
#define ABDAY_2 264
#define ABDAY_3 265
#define ABDAY_4 266
#define ABDAY_5 267
#define ABDAY_6 268
#define ABDAY_7 269
#define MON_1 270
#define MON_2 271
#define MON_3 272
#define MON_4 273
#define MON_5 274
#define MON_6 275
#define MON_7 276
#define MON_8 277
#define MON_9 278
#define MON_10 279
#define MON_11 280
#define MON_12 281
#define ABMON_1 282
#define ABMON_2 283
#define ABMON_3 284
#define ABMON_4 285
#define ABMON_5 286
#define ABMON_6 287
#define ABMON_7 288
#define ABMON_8 289
#define ABMON_9 290
#define ABMON_10 291
#define ABMON_11 292
#define ABMON_12 293
#define D_T_FMT 294
#define D_FMT 295
#define T_FMT 296
#define AM_STR 297
#define PM_STR 298
#define T_FMT_AMPM 299
#define RADIXCHAR 300
#define THOUSEP 301
#define CRNCYSTR 302

#ifdef __cplusplus
extern "C" {
#endif
char *nl_langinfo(nl_item item);
#ifdef __cplusplus
}
#endif
#endif
