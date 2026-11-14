#include <unistd.h>
void * volatile required_symbol = (void *)&lseek;
int main(void) { return required_symbol == 0; }
