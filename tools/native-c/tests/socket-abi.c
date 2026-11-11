#ifdef REVERSE_INCLUDES
#include <unistd.h>
#include <poll.h>
#include <sys/socket.h>
#else
#include <sys/socket.h>
#include <poll.h>
#include <unistd.h>
#endif
#ifdef __cplusplus
#define CHECK static_assert
#else
#define CHECK _Static_assert
#endif
CHECK(sizeof(int) == 4, "native socket argument width");
CHECK(sizeof(ssize_t) == 8, "native I/O return width");
CHECK(AF_INET == 2 && SOCK_STREAM == 1 && IPPROTO_TCP == 6, "native constants");
CHECK(SOCK_NONBLOCK == 0x800 && SOCK_CLOEXEC == 0x80000, "native flags");
static int (*const create_fn)(int, int, int) __attribute__((unused)) = socket;
int main(void) { return 0; }
