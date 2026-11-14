#ifndef SUNLIGHT_SYS_TYPES_H
#define SUNLIGHT_SYS_TYPES_H
#include <stddef.h>
/* Native LP64 public ABI. These types do not advertise additional APIs or
 * define the kernel's private metadata wire layout. No pthread placeholders. */
typedef long ssize_t;
typedef long off_t;
typedef int pid_t;
typedef unsigned int mode_t;
typedef unsigned long dev_t;
typedef unsigned long ino_t;
typedef unsigned int nlink_t;
typedef unsigned int uid_t;
typedef unsigned int gid_t;
typedef long time_t;
typedef long clock_t;
#endif
