#include <string>
std::string& f() { static std::string s("x"); return s; }
int main() { return f().empty(); }
