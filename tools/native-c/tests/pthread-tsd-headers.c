#include <time.h>
#include <pthread.h>
#include <string.h>
#include <errno.h>
#ifdef __cplusplus
#define ASSERT static_assert
#else
#define ASSERT _Static_assert
#endif
ASSERT(sizeof(pthread_key_t) == 8, "LP64 key");
ASSERT(sizeof(pthread_once_t) == 4, "once futex word");
static pthread_once_t control = PTHREAD_ONCE_INIT;
static void init(void) { }
int main(void) {
    int (*reentrant)(int,char *,size_t) = strerror_r;
    pthread_key_t key;
    char buffer[128];
    if (pthread_key_create(&key,0) || pthread_setspecific(key,0) ||
        pthread_getspecific(key) || pthread_key_delete(key) ||
        pthread_once(&control,init)) return 1;
    return reentrant(EINVAL,buffer,sizeof buffer) || strcmp(buffer,strerror(EINVAL));
}
