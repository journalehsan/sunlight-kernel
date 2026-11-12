#include <iostream>
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
    out << "partial\n";
    if (!out.good() || calls != 4) return 1;
    int before = calls;
    out.flush();
    if (calls != before) return 2; // all bytes already reached the backend
    calls = 0; mode = 1;
    out << "abcd";
    if (!out.bad() || calls != 2) return 3;
    out << "not written";
    if (calls != 2) return 4;
    out.clear(); calls = 0; mode = 2;
    out << "xyzz";
    if (!out.bad() || calls != 2) return 5;
    out.clear(); mode = 0;
    out.write(nullptr, 0);
    if (!out.good()) return 6;
    return 0;
}
