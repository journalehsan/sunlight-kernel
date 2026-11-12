#ifndef __ASSEMBLER__
#ifdef __cplusplus
extern "C" [[noreturn]] void __sl_cxx_abort() noexcept;
#else
extern void __sl_cxx_abort(void) __attribute__((noreturn));
#endif
#define abort __sl_cxx_abort
#ifdef __cplusplus
#include <pthread.h>
// Internal libunwind rwlock adaptation. Exclusive native mutex locking for
// readers and writers preserves correctness; no public rwlock API is exposed.
using pthread_rwlock_t = pthread_mutex_t;
#define PTHREAD_RWLOCK_INITIALIZER PTHREAD_MUTEX_INITIALIZER
inline int pthread_rwlock_rdlock(pthread_rwlock_t* m) { return pthread_mutex_lock(m); }
inline int pthread_rwlock_wrlock(pthread_rwlock_t* m) { return pthread_mutex_lock(m); }
inline int pthread_rwlock_unlock(pthread_rwlock_t* m) { return pthread_mutex_unlock(m); }
#endif

#endif
