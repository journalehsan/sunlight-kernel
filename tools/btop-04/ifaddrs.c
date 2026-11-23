#define _GNU_SOURCE
#include <arpa/inet.h>
#include <errno.h>
#include <ifaddrs.h>
#include <netpacket/packet.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char **argv)
{
    for (unsigned call = 0; call < 70; ++call) {
        struct ifaddrs sentinel = {0}, *list = &sentinel;
        if (getifaddrs(&list) != 0) {
            printf("ifaddrs errno=%d unchanged=%d\n", errno, list == &sentinel);
            return argc == 2 && !strcmp(argv[1], "unavailable") &&
                   errno == ENETDOWN && list == &sentinel ? 0 : 1;
        }
        if (argc == 2 || !list || list == &sentinel)
            return 2;
        if (!call) {
            for (struct ifaddrs *p = list; p; p = p->ifa_next) {
                if (!p->ifa_addr)
                    return 3;
                if (p->ifa_addr->sa_family == AF_PACKET) {
                    printf("%s#%d ", p->ifa_name,
                           ((struct sockaddr_ll *)p->ifa_addr)->sll_ifindex);
                } else if (p->ifa_addr->sa_family == AF_INET) {
                    char ip[INET_ADDRSTRLEN], mask[INET_ADDRSTRLEN];
                    if (!p->ifa_netmask ||
                        !inet_ntop(AF_INET, &((struct sockaddr_in *)p->ifa_addr)->sin_addr, ip, sizeof ip) ||
                        !inet_ntop(AF_INET, &((struct sockaddr_in *)p->ifa_netmask)->sin_addr, mask, sizeof mask))
                        return 4;
                    printf("%s=%s/%s ", p->ifa_name, ip, mask);
                }
            }
        }
        freeifaddrs(list);
    }
    puts("cleanup=70");
    return 0;
}
