#include <dlfcn.h>
void * volatile required_symbol = (void *)&dlsym;
int main(void) { return required_symbol == 0; }
