#define _GNU_SOURCE
#include <errno.h>
#include <ifaddrs.h>
#include <stdio.h>
#include <sys/socket.h>

int main(void)
{
    struct ifaddrs sentinel = {0};
    struct ifaddrs *list = &sentinel;
    errno = 0;
    int rc = getifaddrs(&list);
    int error = errno;
    printf("ifaddrs rc=%d errno=%d unchanged=%d\n", rc, error, list == &sentinel);
    if (rc != 0)
        return 1;
    for (const struct ifaddrs *p = list; p; p = p->ifa_next)
        printf("name=%s family=%d flags=%x\n", p->ifa_name,
               p->ifa_addr ? p->ifa_addr->sa_family : -1, p->ifa_flags);
    freeifaddrs(list);
    puts("ifaddrs cleanup ok");
    return 0;
}
