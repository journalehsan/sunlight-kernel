#include <string>
#include <unistd.h>
static int mode, calls;
extern "C" ssize_t __real_write(int, const void*, size_t);
extern "C" ssize_t __wrap_write(int fd, const void* data, size_t size) {
    ++calls;
    if (mode == 1 && calls > 1) return 0;
    if (mode == 2 && calls > 1) return __real_write(-1, data, size);
    return __real_write(fd, data, size > 2 ? 2 : size);
}
int main() {
    std::ostream out(1);
    out << std::string("A\0BCD", 5);
    if (!out.good() || calls != 3) return 1;
    calls = 0; mode = 1;
    out << std::string("zero");
    if (!out.bad() || calls != 2) return 2;
    out << std::string("suppressed");
    if (calls != 2) return 3;
    out.clear(); calls = 0; mode = 2;
    out << std::string("error");
    if (!out.bad() || calls != 2) return 4;
    out.clear(); calls = 0; mode = 0;
    out << std::string();
    if (!out.good() || calls) return 5;
    return 0;
}
