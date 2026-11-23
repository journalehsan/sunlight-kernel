#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <unistd.h>

int main(int argc, char **argv)
{
    const char *path = argc == 2 ? argv[1] : "/mnt/READ10.BIN";
    int fd = open(path, O_RDONLY);
    if (fd < 0) {
        int error = errno;
        printf("READ10 open errno=%d\n", error);
        return error == ENOENT ? 3 : 1;
    }
    unsigned char buffer[4096];
    size_t offset = 0;
    while (offset < 65536) {
        ssize_t count = read(fd, buffer, sizeof buffer);
        if (count <= 0)
            return 1;
        for (ssize_t index = 0; index < count; index++, offset++) {
            unsigned char expected = (offset * 37 + (offset >> 8) * 17 + 11) & 255;
            if (buffer[index] != expected)
                return 2;
        }
    }
    if (offset != 65536 || read(fd, buffer, 1) != 0 || close(fd) != 0)
        return 1;
    printf("READ10 verified bytes=%zu EOF=1\n", offset);
    return 0;
}
