#ifndef SUNLIGHT_DIRENT_H
#define SUNLIGHT_DIRENT_H

/* Native subset: "." and canonical absolute paths; stable directories only.
 * No directory descriptor, inode, seek, or concurrent mutation contract.
 * Other pathname forms fail ENOSYS. Names reflect the native 64-byte ABI.
 * A returned entry belongs to its stream until the next readdir/closedir.
 * Callers must serialize operations on the same live stream. */
typedef struct SunlightDir DIR;
struct dirent {
    char d_name[65];
};
#ifdef __cplusplus
extern "C" {
#endif
DIR *opendir(const char *path);
struct dirent *readdir(DIR *stream);
int closedir(DIR *stream);
#ifdef __cplusplus
}
#endif
#endif
