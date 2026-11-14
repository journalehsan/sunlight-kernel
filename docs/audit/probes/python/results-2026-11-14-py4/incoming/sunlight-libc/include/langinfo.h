#ifndef SUNLIGHT_LANGINFO_H
#define SUNLIGHT_LANGINFO_H

/* Native subset: the initial, fixed C locale uses US-ASCII. LANG/LC_*
 * environment variables do not select a locale. Only CODESET is exposed;
 * other item values return an empty string. Returned storage has process
 * lifetime and must not be modified or freed. No locale objects are exposed.
 * nl_item values are native ABI constants, not a foreign libc's encoding. */
typedef int nl_item;
#define CODESET 1

#ifdef __cplusplus
extern "C" {
#endif
char *nl_langinfo(nl_item item);
#ifdef __cplusplus
}
#endif
#endif
