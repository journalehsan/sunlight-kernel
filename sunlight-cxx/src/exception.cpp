#include <stdexcept>
using size_t = __SIZE_TYPE__;
extern "C" {
void* malloc(size_t) noexcept;
void free(void*) noexcept;
size_t strlen(const char*) noexcept;
void* memcpy(void*, const void*, size_t) noexcept;
[[noreturn]] void sl_std_exit(int) noexcept;
}
namespace std {
exception::~exception() noexcept = default;
const char* exception::what() const noexcept { return "std::exception"; }
struct runtime_error::message { size_t references; };
runtime_error::runtime_error(const char* s, size_t n) noexcept : message_(nullptr) {
    if (!s) sl_std_exit(134);
    if (n > __SIZE_MAX__ - sizeof(message) - 1) sl_std_exit(134);
    message_ = static_cast<message*>(::malloc(sizeof(message) + n + 1));
    if (!message_) sl_std_exit(134);
    message_->references = 1;
    ::memcpy(message_ + 1, s, n);
    reinterpret_cast<char*>(message_ + 1)[n] = 0;
}
runtime_error::runtime_error(const char* s) noexcept
    : runtime_error(s, s ? ::strlen(s) : 0) {}
runtime_error::runtime_error(const string& s) noexcept : runtime_error(s.data(), s.size()) {}
runtime_error::runtime_error(const runtime_error& s) noexcept : exception(s), message_(s.message_) {
    if (__atomic_fetch_add(&message_->references, 1, __ATOMIC_RELAXED) == __SIZE_MAX__) sl_std_exit(134);
}
runtime_error& runtime_error::operator=(const runtime_error& s) noexcept {
    if (this != &s) {
        runtime_error copy(s);
        message* old = message_;
        message_ = copy.message_;
        copy.message_ = old;
    }
    return *this;
}
runtime_error::~runtime_error() noexcept {
    if (__atomic_sub_fetch(&message_->references, 1, __ATOMIC_ACQ_REL) == 0) ::free(message_);
}
const char* runtime_error::what() const noexcept { return reinterpret_cast<const char*>(message_ + 1); }
}
