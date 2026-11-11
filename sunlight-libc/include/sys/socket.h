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
/* AF_UNIX SOCK_STREAM only. The two descriptors share a full-duplex
 * socketpair; MSG_OOB publishes one urgent byte in the selected direction. */
int socketpair(int domain, int type, int protocol, int fds[2]);
ssize_t send(int fd, const void *buffer, size_t length, int flags);
ssize_t recv(int fd, void *buffer, size_t length, int flags);
#ifdef __cplusplus
}
#endif
#endif
