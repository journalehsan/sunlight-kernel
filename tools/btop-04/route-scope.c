#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/socket.h>
#include <unistd.h>

enum { NETLINK_ROUTE = 0, NETLINK_USERSOCK = 2, RTM_GETLINK = 18,
       RTM_NEWADDR = 20, RTM_GETROUTE = 26 };
struct request {
    struct {
        uint32_t nlmsg_len;
        uint16_t nlmsg_type, flags;
        uint32_t seq, pid;
    } header;
    uint32_t body;
};
_Static_assert(sizeof(struct request) == 20, "netlink wire size");

static int proc_counters(unsigned long long *rx, unsigned long long *tx)
{
    FILE *file = fopen("/proc/net/dev", "r");
    if (!file)
        return -1;
    char line[256];
    int valid = 0;
    while (fgets(line, sizeof line, file)) {
        unsigned long long row[9];
        if (sscanf(line, " eth0: %llu %llu %llu %llu %llu %llu %llu %llu %llu",
                   &row[0], &row[1], &row[2], &row[3], &row[4], &row[5], &row[6],
                   &row[7], &row[8]) == 9) {
            *rx = row[0];
            *tx = row[8];
            valid = 1;
            break;
        }
    }
    if (fclose(file) != 0 || !valid)
        return -1;
    return 0;
}

static int attribute(const char *path, unsigned long long *value)
{
    FILE *file = fopen(path, "r");
    if (!file)
        return -1;
    int fields = fscanf(file, "%llu", value);
    if (fclose(file) != 0 || fields != 1)
        return -1;
    return 0;
}

int main(void)
{
    if (socket(AF_NETLINK, SOCK_RAW, NETLINK_USERSOCK) != -1 || errno != EPROTONOSUPPORT)
        return 1;
    int fd = socket(AF_NETLINK, SOCK_RAW | SOCK_CLOEXEC, NETLINK_ROUTE);
    if (fd < 0)
        return 2;
    struct request req = {{20, RTM_NEWADDR, 0x301, 7, 0}, 0};
    if (send(fd, &req, sizeof req, 0) != -1 || errno != EOPNOTSUPP)
        return 3;
    req.header.nlmsg_type = RTM_GETROUTE;
    if (send(fd, &req, sizeof req, 0) != -1 || errno != EOPNOTSUPP)
        return 4;
    req.header.nlmsg_type = RTM_GETLINK;
    req.header.nlmsg_len = 19;
    if (send(fd, &req, sizeof req, 0) != -1 || errno != EINVAL)
        return 5;
    req.header.nlmsg_len = 20;
    if (send(fd, &req, sizeof req, 0) != 20)
        return 6;
    unsigned char reply[128];
    if (recv(fd, reply, 1, MSG_DONTWAIT) != -1 || errno != EMSGSIZE ||
        recv(fd, reply, sizeof reply, MSG_DONTWAIT) <= 0 ||
        recv(fd, reply, sizeof reply, MSG_DONTWAIT) != -1 || errno != EAGAIN)
        return 7;
    if (close(fd))
        return 8;
    const char *rx = "/sys/class/net/eth0/statistics/rx_bytes";
    const char *tx = "/sys/class/net/eth0/statistics/tx_bytes";
    unsigned long long before_rx, before_tx, after_rx, after_tx, sys_rx, sys_tx;
    if (open(rx, O_WRONLY) != -1 || errno != EROFS ||
        open("/sys/class/net/lo/statistics/rx_bytes", O_RDONLY) != -1 || errno != ENOENT)
        return 9;
    if (proc_counters(&before_rx, &before_tx) || attribute(rx, &sys_rx) ||
        attribute(tx, &sys_tx) || proc_counters(&after_rx, &after_tx) ||
        sys_rx < before_rx || sys_rx > after_rx || sys_tx < before_tx || sys_tx > after_tx)
        return 10;
    printf("scope PASS sysfs rx=%llu tx=%llu within procfs [%llu,%llu] [%llu,%llu]\n",
           sys_rx, sys_tx, before_rx, after_rx, before_tx, after_tx);
    return 0;
}
