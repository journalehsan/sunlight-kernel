#include <iostream>
#include <unistd.h>
#include <limits.h>
int main() {
    std::cout << "output " << 0 << ' ' << true << ' ' << LLONG_MIN << ' '
              << LLONG_MAX << ' ' << ULLONG_MAX << ' ' << ULONG_MAX << std::endl;
    std::cout << "bounds " << static_cast<short>(SHRT_MIN) << ' '
              << static_cast<short>(SHRT_MAX) << ' ' << static_cast<unsigned short>(USHRT_MAX)
              << ' ' << INT_MIN << ' ' << INT_MAX << ' ' << UINT_MAX << ' '
              << LONG_MIN << ' ' << LONG_MAX << ' ' << false << '\n';
    std::cerr << "stderr" << std::flush << '\n';
    std::clog << "clog\n";
    if (!std::cout.good() || !std::cerr.good() || !std::clog.good()) return 1;
    // Flush has no queued bytes; direct write ordering witnesses submission.
    std::cout << "A" << std::flush;
    if (write(1, "B", 1) != 1) return 2;
    std::cout << "C\n";
    std::ostream invalid(-1);
    invalid << "rejected";
    if (!invalid.bad() || !invalid.fail() || invalid) return 3;
    invalid.flush();
    if (!invalid.bad()) return 4;
    invalid.clear();
    if (!invalid.good()) return 5;
    invalid.write(nullptr, 1);
    if (!invalid.bad()) return 6;
    std::ostream badsize(1);
    badsize.write("x", -1);
    if (!badsize.bad()) return 7;
    if (close(1)) return 8;
    std::cout << "closed" << std::flush;
    return std::cout.bad() ? 0 : 9;
}
