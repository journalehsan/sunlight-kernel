#include <string>
#include <string.h>
#include <stdlib.h>
static unsigned allocations, frees;
extern "C" void* __real_malloc(size_t);
extern "C" void __real_free(void*);
extern "C" void* __wrap_malloc(size_t n) { ++allocations; return __real_malloc(n); }
extern "C" void __wrap_free(void* p) { if (p) ++frees; __real_free(p); }
#define CHECK(c) do { if (!(c)) return __LINE__; } while (0)
static std::string moved(std::string&& s) noexcept { return static_cast<std::string&&>(s); }
int main() {
    {
        std::string empty;
        CHECK(empty.empty() && empty.size() == 0 && empty.length() == 0);
        CHECK(empty.data() && empty.c_str()[0] == 0 && empty.capacity() == 0);
        empty[0] = 0;
        std::string null_range(nullptr, 0);
        CHECK(null_range == empty);
        std::string a("abc"), b(a);
        a[0] = 'A';
        CHECK(b == "abc" && a == "Abc" && a.c_str()[3] == 0);
        std::string c = moved(static_cast<std::string&&>(a));
        CHECK(c == "Abc" && a.empty() && a.c_str()[0] == 0);
        a += "reused";
        CHECK(a == "reused");
        const std::string& same = c;
        c = same;
        CHECK(c == "Abc");
        std::string& move_alias = c;
        c = static_cast<std::string&&>(move_alias);
        CHECK(c == "Abc");
        c = b;
        b[1] = 'B';
        CHECK(c == "abc");
        c = c.c_str() + 1;
        CHECK(c == "bc");
        c += c; // growth with alias
        CHECK(c == "bcbc");
        c.append(c.data() + 1, 3); // growth with interior alias
        CHECK(c == "bcbccbc");
        CHECK(c.capacity() >= c.size());
        c.append(c.data(), 1); // spare capacity, overlap-safe
        CHECK(c == "bcbccbcb" && c.c_str()[c.size()] == 0);
        std::string term("abc");
        term.append(term.c_str() + term.size(), 1); // include old terminator
        CHECK(term.size() == 4 && term[3] == 0 && term[4] == 0);
        std::string binary("A\0B", 3), copy(binary);
        CHECK(binary.size() == 3 && copy[1] == 0 && copy[2] == 'B' && copy[3] == 0);
        copy.data()[2] = 'C';
        CHECK(binary[2] == 'B' && binary < copy && binary != copy);
        CHECK(std::string("\xff", 1) > std::string("\x7f", 1));
        CHECK(std::string("a") < std::string("aa"));
        CHECK(binary <= binary && binary >= binary && binary.compare(binary) == 0);
        std::string sum = std::string("prefix") + binary;
        CHECK(sum.size() == 9 && sum[7] == 0 && sum[8] == 'B');
        std::string target("old allocation");
        target = static_cast<std::string&&>(sum);
        CHECK(target.size() == 9 && sum.empty() && sum.c_str()[0] == 0);
        target = std::string();
        CHECK(target.empty() && target.c_str()[0] == 0);
        for (unsigned iteration = 0; iteration < 2000; ++iteration) {
            std::string growing;
            for (unsigned i = 0; i < 4097; ++i) growing += static_cast<char>('a' + i % 26);
            CHECK(growing.size() == 4097 && growing.capacity() >= 4097);
            for (unsigned i = 0; i < 4097; ++i) CHECK(growing[i] == 'a' + i % 26);
            CHECK(growing.c_str()[4097] == 0);
            std::string independent(growing);
            growing[0] = 'Z';
            CHECK(independent[0] == 'a');
        }
        std::cout << std::string("string:") << binary << ':' << empty << 42 << std::endl;
        CHECK(std::cout.good());
    }
    CHECK(allocations > 2000 && allocations == frees);
    return 0;
}
