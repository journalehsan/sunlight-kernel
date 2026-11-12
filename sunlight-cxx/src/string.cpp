#include <string>
// The native C ABI implementation does not unwind. Private declarations avoid
// importing host C++ headers or changing libc's public C interface.
using size_t = __SIZE_TYPE__;
extern "C" {
size_t strlen(const char*) noexcept;
void* memcpy(void*, const void*, size_t) noexcept;
void* memmove(void*, const void*, size_t) noexcept;
int memcmp(const void*, const void*, size_t) noexcept;
void* malloc(size_t) noexcept;
void free(void*) noexcept;
[[noreturn]] void sl_std_exit(int) noexcept;
}
namespace {
[[noreturn]] void string_failure() noexcept {
    // Raw native termination: no atexit callbacks against partially built state.
    sl_std_exit(134);
}
}
namespace std {
string::string(const char* s) noexcept : string() {
    if (!s) string_failure();
    append(s, ::strlen(s));
}
string::string(const char* s, size_type n) noexcept : string() { append(s, n); }
string::string(const string& s) noexcept : string(s.data(), s.size()) {}
string::string(string&& s) noexcept : string() { *this = static_cast<string&&>(s); }
void string::release() noexcept {
    ::free(bytes_);
    bytes_ = nullptr;
    length_ = capacity_ = 0;
    empty_ = 0;
}
string::~string() noexcept { ::free(bytes_); }
string& string::operator=(string&& s) noexcept {
    if (this != &s) {
        release();
        bytes_ = s.bytes_;
        length_ = s.length_;
        capacity_ = s.capacity_;
        s.bytes_ = nullptr;
        s.length_ = s.capacity_ = 0;
        s.empty_ = 0;
    }
    return *this;
}
string& string::operator=(const string& s) noexcept {
    if (this != &s) {
        string copy(s);
        *this = static_cast<string&&>(copy);
    }
    return *this;
}
string& string::operator=(const char* s) noexcept {
    // Copy first: s may be a suffix of this object's allocation.
    string copy(s);
    return *this = static_cast<string&&>(copy);
}
string& string::append(const char* s) noexcept {
    if (!s) string_failure();
    return append(s, ::strlen(s));
}
string& string::append(const char* s, size_type n) noexcept {
    if (n > max_size() - length_ || (!s && n)) string_failure();
    if (!n) return *this;
    const size_type total = length_ + n;
    if (total > capacity_) {
        size_type grown = capacity_ <= max_size() / 2 ? capacity_ * 2 : max_size();
        if (grown < total) grown = total;
        char* next = static_cast<char*>(::malloc(grown + 1));
        if (!next) string_failure();
        if (length_) ::memcpy(next, data(), length_);
        // Old storage remains live while reading aliased source bytes.
        ::memcpy(next + length_, s, n);
        next[total] = 0;
        ::free(bytes_);
        bytes_ = next;
        capacity_ = grown;
    } else {
        ::memmove(bytes_ + length_, s, n);
        bytes_[total] = 0;
    }
    length_ = total;
    return *this;
}
int string::compare(const string& s) const noexcept {
    size_type n = length_ < s.length_ ? length_ : s.length_;
    int c = ::memcmp(data(), s.data(), n);
    return c ? c : (length_ > s.length_) - (length_ < s.length_);
}
string operator+(const string& a, const string& b) noexcept {
    string result(a);
    result.append(b);
    return result;
}
bool operator==(const string& a, const string& b) noexcept { return a.compare(b) == 0; }
bool operator!=(const string& a, const string& b) noexcept { return a.compare(b) != 0; }
bool operator<(const string& a, const string& b) noexcept { return a.compare(b) < 0; }
bool operator>(const string& a, const string& b) noexcept { return a.compare(b) > 0; }
bool operator<=(const string& a, const string& b) noexcept { return a.compare(b) <= 0; }
bool operator>=(const string& a, const string& b) noexcept { return a.compare(b) >= 0; }
ostream& operator<<(ostream& out, const string& s) noexcept {
    return out.write(s.data(), static_cast<long>(s.size()));
}
}
