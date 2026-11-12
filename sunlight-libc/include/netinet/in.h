#ifndef SUNLIGHT_NETINET_IN_H
#define SUNLIGHT_NETINET_IN_H
#ifdef __cplusplus
extern "C" {
#endif

typedef unsigned short in_port_t;
typedef unsigned int in_addr_t;

struct in_addr {
    in_addr_t s_addr;
};

/* Standard 16-byte IPv4 socket address. sin_port and sin_addr are stored in
 * network byte order; the native transport bridge reads the bytes directly. */
struct sockaddr_in {
    unsigned short sin_family;
    in_port_t sin_port;
    struct in_addr sin_addr;
    unsigned char sin_zero[8];
};

#define INADDR_ANY ((in_addr_t)0x00000000u)
/* Host-order 127.0.0.1, exactly like the BSD constant. Callers pass it through
 * htonl() before storing it in sin_addr; the bridge reads the resulting memory
 * bytes as the network-order octets. */
#define INADDR_LOOPBACK ((in_addr_t)0x7f000001u)

unsigned short htons(unsigned short value);
unsigned short ntohs(unsigned short value);
unsigned int htonl(unsigned int value);
unsigned int ntohl(unsigned int value);

#ifdef __cplusplus
}
#endif
#endif
