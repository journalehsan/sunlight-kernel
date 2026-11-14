#include <pthread.h>
#include <errno.h>
#include <string.h>
#include <stdint.h>
#include <stdio.h>
static const int errors[]={0,EPERM,ENOENT,EIO,E2BIG,EBADF,EAGAIN,ENOMEM,EACCES,EFAULT,EBUSY,EEXIST,ENOTDIR,EISDIR,EINVAL,EMFILE,ENOTTY,ENOSPC,EPIPE,EDOM,ERANGE,EDEADLK,ENOSYS,EILSEQ,EPROTONOSUPPORT,ESOCKTNOSUPPORT,EOPNOTSUPP,EAFNOSUPPORT,EADDRINUSE,ENETUNREACH,ECONNABORTED,ECONNRESET,ENOBUFS,EISCONN,ENOTCONN,ETIMEDOUT,ECONNREFUSED,EHOSTUNREACH,EALREADY,EINPROGRESS};
static void *worker(void *arg) {
    int own=(int)(uintptr_t)arg; errno=own;
    for(int iteration=0;iteration<1000;iteration++) for(unsigned i=0;i<sizeof errors/sizeof errors[0];i++) {
        char buffer[128]; char *stable=strerror(errors[i]);
        if(!stable || strcmp(stable,"Unknown error")==0 || strerror_r(errors[i],buffer,sizeof buffer) || strcmp(stable,buffer)) return (void *)1;
        if(strcmp(strerror(-999),"Unknown error") || strerror(errors[i])!=stable || errno!=own) return (void *)2;
    }
    return 0;
}
int main(void) {
    pthread_t threads[4]; void *result;
    for(uintptr_t i=0;i<4;i++) if(pthread_create(&threads[i],0,worker,(void *)(i+100))) return 1;
    for(int i=0;i<4;i++) if(pthread_join(threads[i],&result) || result) return 2;
    puts("strerror all 40 supported numbers across four threads PASS");
    return 0;
}
