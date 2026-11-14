#include <sys/mman.h>
void * volatile required_symbol = (void *)&mmap;
int main(void) { return required_symbol == 0; }
