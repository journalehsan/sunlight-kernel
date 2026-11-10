#ifndef SUNLIGHT_SYS_MMAN_H
#define SUNLIGHT_SYS_MMAN_H
#include <stddef.h>
#ifndef SUNLIGHT_OFF_T_DEFINED
#define SUNLIGHT_OFF_T_DEFINED
typedef long off_t;
#endif
#define PROT_NONE 0
#define PROT_READ 1
#define PROT_WRITE 2
#define PROT_EXEC 4
#define MAP_SHARED 0x01
#define MAP_PRIVATE 0x02
#define MAP_FIXED 0x10
#define MAP_ANONYMOUS 0x20
#define MAP_FAILED ((void *)-1)
#ifdef __cplusplus
extern "C" {
#endif
/* Native LP64 subset: private anonymous mappings, fd=-1, offset=0.
 * Require READ, READ|WRITE or READ|EXEC; NONE, W|X and unknown bits fail.
 * Nonfixed addr must be NULL. FIXED requires a non-NULL page-aligned addr
 * and replaces eligible anonymous mappings at that exact address.
 * Length is rounded to 4096 bytes; zero and overflow fail. File/shared
 * mappings and hints are unsupported. Constants do not imply support.
 * Failure returns MAP_FAILED/-1 and sets errno; success preserves errno.
 * Native kernel failures currently report generic EIO. */
void *mmap(void *addr, size_t length, int prot, int flags, int fd, off_t offset);
int munmap(void *addr, size_t length);
/* Present private anonymous pages only, same protection subset as mmap.
 * Full-range checks precede page-permission updates and TLB invalidation.
 * addr must be non-NULL and page-aligned. Aligned zero length succeeds.
 * No arbitrary user mapping or file-backed protection support is promised. */
int mprotect(void *addr, size_t length, int prot);
#ifdef __cplusplus
}
#endif
#endif
