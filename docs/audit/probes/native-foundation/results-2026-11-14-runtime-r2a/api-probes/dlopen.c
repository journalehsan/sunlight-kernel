#include <dlfcn.h>
void * volatile required_symbol = (void *)&dlopen;
int main(void) { return required_symbol == 0; }
