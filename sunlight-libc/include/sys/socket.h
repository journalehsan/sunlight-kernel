#ifndef SUNLIGHT_SYS_SOCKET_H
#define SUNLIGHT_SYS_SOCKET_H
#include <stddef.h>
#include <unistd.h>
#ifdef __cplusplus
extern "C" {
#endif
#define AF_UNIX 1
#define AF_INET 2
/* Native creation-only IPv4 stream endpoint; protocol 0 or IPPROTO_TCP. */
#define IPPROTO_TCP 6
int socket(int domain, int type, int protocol);
#define SOCK_STREAM 1
#define SOCK_NONBLOCK 0x00000800
#define SOCK_CLOEXEC  0x00080000
#define MSG_OOB       0x01
#define MSG_DONTWAIT  0x40
typedef unsigned int socklen_t;
struct sockaddr {
    unsigned short sa_family;
    char sa_data[14];
};
#define SHUT_RD   0
#define SHUT_WR   1
#define SHUT_RDWR 2
/* AF_UNIX SOCK_STREAM only. The two descriptors share a full-duplex
 * socketpair; MSG_OOB publishes one urgent byte in the selected direction. */
int socketpair(int domain, int type, int protocol, int fds[2]);
ssize_t send(int fd, const void *buffer, size_t length, int flags);
ssize_t recv(int fd, void *buffer, size_t length, int flags);
/* AF_INET/SOCK_STREAM transport attachment through the `net` service.
 * Blocking calls use bounded deadlines; MSG_DONTWAIT and O_NONBLOCK are
 * honoured. shutdown() is explicitly unsupported (EOPNOTSUPP). */
int bind(int fd, const struct sockaddr *addr, socklen_t len);
int connect(int fd, const struct sockaddr *addr, socklen_t len);
int listen(int fd, int backlog);
int accept(int fd, struct sockaddr *addr, socklen_t *len);
int getsockname(int fd, struct sockaddr *addr, socklen_t *len);
int getpeername(int fd, struct sockaddr *addr, socklen_t *len);
int shutdown(int fd, int how);
ssize_t sendto(int fd, const void *buffer, size_t length, int flags,
               const struct sockaddr *addr, socklen_t addr_len);
ssize_t recvfrom(int fd, void *buffer, size_t length, int flags,
                 struct sockaddr *addr, socklen_t *addr_len);
#ifdef __cplusplus
}
#endif
#endif
