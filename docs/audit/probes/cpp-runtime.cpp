#include <iostream>
#include <string>
#include <stdexcept>
#include <thread>
#include <typeinfo>
#include <cstdlib>
struct Base { virtual ~Base() = default; }; struct Derived : Base {};
std::string initialized("static initialization OK");
thread_local std::string local("TLS OK");
static void done() { std::cout << "atexit OK\n"; }
int main() {
    std::atexit(done);
    Base *b = new Derived;
    if (!dynamic_cast<Derived*>(b) || typeid(*b)!=typeid(Derived)) return 1;
    delete b;
    bool caught=false;
    try { throw std::runtime_error("exception OK"); }
    catch (const std::exception &e) { caught=true; std::cout << e.what() << '\n'; }
    std::thread t([] { std::cout << local << '\n'; }); t.join();
    std::cout << initialized << '\n';
    return !caught;
}
