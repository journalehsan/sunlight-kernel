#include <stdio.h>
#include <errno.h>
int main(void) {
 errno=0;FILE *f=fopen("/dev/ptmx","r");if(f||errno!=ENOENT)return 1;
 errno=0;f=fopen("/dev/ptc","r");if(f||errno!=ENOENT)return 2;
 puts("R2A /dev/ptmx and /dev/ptc absent: ENOENT");return 0;
}
