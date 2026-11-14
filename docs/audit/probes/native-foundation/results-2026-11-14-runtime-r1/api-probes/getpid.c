#include <unistd.h>
void * volatile required_symbol = (void *)&getpid;
int main(void) { return required_symbol == 0; }
