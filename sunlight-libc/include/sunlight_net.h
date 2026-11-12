#ifndef SUNLIGHT_NET_H
#define SUNLIGHT_NET_H
#ifdef __cplusplus
extern "C" {
#endif

/* Non-standard bounded diagnostic accessor for native networking fixtures.
 * Selectors mirror `sunlight_ipc::netop::NetDiagnostic`; returns -1 when the
 * `net` service is unavailable. This is not a POSIX socket option. */
#define SUNLIGHT_NET_DIAGNOSTIC_SOCKET_LIVE 2
#define SUNLIGHT_NET_DIAGNOSTIC_LISTENER_LIVE 6
#define SUNLIGHT_NET_DIAGNOSTIC_STREAM_LIVE 9
long sunlight_net_diagnostic(int selector);

#ifdef __cplusplus
}
#endif
#endif
