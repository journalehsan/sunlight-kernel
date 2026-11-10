#ifndef SUNLIGHT_DLFCN_H
#define SUNLIGHT_DLFCN_H
#define RTLD_NOW 2
#ifdef __cplusplus
extern "C" {
#endif
/* Only the main program and RTLD_NOW are supported. No shared objects.
 * Lookup covers the native linker's explicit function export index.
 * Opens share a reference-counted main namespace; close returns 0 or -1.
 * Unsupported opens and missing/invalid lookups return NULL; no dlerror API.
 */
void *dlopen(const char *path, int flags);
void *dlsym(void *handle, const char *name);
int dlclose(void *handle);
#ifdef __cplusplus
}
#endif
#endif
