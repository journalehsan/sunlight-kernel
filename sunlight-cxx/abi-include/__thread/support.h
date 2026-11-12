#ifndef SUNLIGHT_CXX_THREAD_SUPPORT_H
#define SUNLIGHT_CXX_THREAD_SUPPORT_H
/* Private subset of libc++'s <__thread/support.h> for the vendored guard
 * implementation. The selected guard implementation is the futex one; the
 * mutex/condition-variable types below only keep the unselected GlobalMutex
 * template well-formed. The condition-variable entry points are declared but
 * deliberately not defined, so selecting that path fails at link time rather
 * than silently degrading to an abort stub. */
#include <pthread.h>

#define _LIBCPP_HAS_THREAD_API_PTHREAD 1

#ifndef _LIBCPP_CONSTINIT
#define _LIBCPP_CONSTINIT
#endif

#define _LIBCPP_MUTEX_INITIALIZER PTHREAD_MUTEX_INITIALIZER
#define _LIBCPP_CONDVAR_INITIALIZER { 0 }

namespace std {
using __libcpp_mutex_t = pthread_mutex_t;
struct __libcpp_condvar_t {
    unsigned int __word;
};

inline int __libcpp_mutex_lock(__libcpp_mutex_t* mutex) {
    return pthread_mutex_lock(mutex);
}
inline int __libcpp_mutex_unlock(__libcpp_mutex_t* mutex) {
    return pthread_mutex_unlock(mutex);
}
int __libcpp_condvar_wait(__libcpp_condvar_t*, __libcpp_mutex_t*);
int __libcpp_condvar_broadcast(__libcpp_condvar_t*);
}

#endif
