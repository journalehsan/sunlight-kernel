#include <iostream>
using size_t = __SIZE_TYPE__;
using ssize_t = long;
extern "C" ssize_t write(int, const void*, size_t) noexcept;
namespace std {
// Constant initialization: no guard, heap, ABI runtime or init-order dependency.
ostream cout(1), cerr(2), clog(2);
ostream& ostream::write(const char* data, long size) noexcept {
    if (fail()) return *this;
    if (size < 0 || (!data && size)) { setstate(badbit); return *this; }
    while (size) {
        ssize_t n = ::write(fd_, data, static_cast<size_t>(size));
        if (n <= 0 || n > size) { setstate(badbit); break; }
        data += n;
        size -= n;
    }
    return *this;
}
ostream& ostream::put(char c) noexcept { return write(&c, 1); }
ostream& ostream::operator<<(const char* s) noexcept {
    if (!s) { setstate(badbit); return *this; }
    const char* end = s;
    while (*end) ++end;
    return write(s, end - s);
}
ostream& ostream::operator<<(unsigned long long v) noexcept {
    char digits[20];
    char* end = digits + sizeof digits;
    char* p = end;
    do { *--p = static_cast<char>('0' + v % 10); v /= 10; } while (v);
    return write(p, end - p);
}
ostream& ostream::operator<<(long long v) noexcept {
    // Unsigned subtraction handles LLONG_MIN without signed overflow.
    if (v < 0) put('-');
    return *this << (v < 0 ? 0ULL - static_cast<unsigned long long>(v)
                           : static_cast<unsigned long long>(v));
}
ostream& ostream::flush() noexcept {
    // Unbuffered descriptor backend: every byte has already been submitted.
    // No fsync promise; flush has no pending bytes and preserves error state.
    return *this;
}
ostream& endl(ostream& s) noexcept { return s.put('\n').flush(); }
ostream& flush(ostream& s) noexcept { return s.flush(); }
}
