#define _GNU_SOURCE
#include <sys/mman.h>
int main(void){void *p=mmap(0,1048576,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0); if(p==MAP_FAILED)return 1; ((char*)p)[1048575]=1; return mprotect(p,1048576,PROT_READ)||munmap(p,1048576);}
