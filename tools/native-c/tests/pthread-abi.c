#ifdef REVERSE_INCLUDES
#include <stddef.h>
#endif
#include <pthread.h>
#include <pthread.h>
#ifndef REVERSE_INCLUDES
#include <stddef.h>
#endif
#ifdef __cplusplus
template <class A, class B> struct same { static const bool value = false; };
template <class A> struct same<A, A> { static const bool value = true; };
#define TYPE(expr, type) static_assert((same<decltype(expr), type>::value), #expr)
#define ASSERT static_assert
#else
#define TYPE(expr, type) _Static_assert(_Generic((expr), type: 1, default: 0), #expr)
#define ASSERT _Static_assert
#endif
TYPE((pthread_t)0, struct __sunlight_pthread *);
TYPE(((pthread_mutex_t *)0)->__lock, unsigned int);
TYPE(((pthread_attr_t *)0)->__detachstate, int);
TYPE(&pthread_create, int (*)(pthread_t *, const pthread_attr_t *,
                              void *(*)(void *), void *));
TYPE(&pthread_join, int (*)(pthread_t, void **));
#ifdef __cplusplus
static_assert(same<decltype(pthread_exit((void *)0)), void>::value,
              "pthread_exit returns void");
#else
TYPE(&pthread_exit, void (*)(void *));
#endif
TYPE(&pthread_self, pthread_t (*)(void));
TYPE(&pthread_equal, int (*)(pthread_t, pthread_t));
TYPE(&pthread_detach, int (*)(pthread_t));
TYPE(&pthread_attr_init, int (*)(pthread_attr_t *));
TYPE(&pthread_attr_destroy, int (*)(pthread_attr_t *));
TYPE(&pthread_attr_setdetachstate, int (*)(pthread_attr_t *, int));
TYPE(&pthread_attr_getdetachstate, int (*)(const pthread_attr_t *, int *));
TYPE(&pthread_mutex_init, int (*)(pthread_mutex_t *, const void *));
TYPE(&pthread_mutex_destroy, int (*)(pthread_mutex_t *));
TYPE(&pthread_mutex_lock, int (*)(pthread_mutex_t *));
TYPE(&pthread_mutex_trylock, int (*)(pthread_mutex_t *));
TYPE(&pthread_mutex_unlock, int (*)(pthread_mutex_t *));
ASSERT(sizeof(pthread_t) == 8, "pthread_t pointer size");
ASSERT(sizeof(pthread_mutex_t) == 4, "mutex size");
ASSERT(__alignof__(pthread_mutex_t) == 4, "mutex alignment");
ASSERT(offsetof(pthread_mutex_t, __lock) == 0, "mutex lock offset");
ASSERT(sizeof(pthread_attr_t) == 4, "attr size");
ASSERT(__alignof__(pthread_attr_t) == 4, "attr alignment");
ASSERT(offsetof(pthread_attr_t, __detachstate) == 0, "attr detachstate offset");
ASSERT(PTHREAD_CREATE_JOINABLE == 0 && PTHREAD_CREATE_DETACHED == 1,
       "detach state ABI");
static pthread_mutex_t initializer = PTHREAD_MUTEX_INITIALIZER;
int main(void) {
    return initializer.__lock == 0 ? 0 : 1;
}
