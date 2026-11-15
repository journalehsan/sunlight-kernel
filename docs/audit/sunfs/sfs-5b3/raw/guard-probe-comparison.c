#define _GNU_SOURCE
#include <sys/mman.h>
#include <sys/uio.h>
#include <unistd.h>
#include <errno.h>
#include <stdio.h>
#include <fcntl.h>
int main(void) {
 char c; int fds[2];
 void *p=mmap(0,4096,PROT_READ|PROT_WRITE,MAP_PRIVATE|MAP_ANONYMOUS,-1,0);
 struct iovec l={&c,1}, r={p,1};
 if(p==MAP_FAILED || pipe2(fds,O_NONBLOCK|O_CLOEXEC))return 1;
 errno=0; ssize_t valid=write(fds[1],p,1); int ve=errno;
 if(mprotect(p,4096,PROT_NONE))return 2;
 errno=0; ssize_t a=process_vm_readv(getpid(),&l,1,&r,1,0);int ae=errno;
 errno=0; ssize_t b=write(fds[1],p,1);int be=errno;
 printf("before-protect pipe-write=%zd errno=%d after-protect process-vm-readv=%zd errno=%d pipe-write=%zd errno=%d\n",valid,ve,a,ae,b,be);
 return !(valid==1 && b==-1 && be==EFAULT);
}
