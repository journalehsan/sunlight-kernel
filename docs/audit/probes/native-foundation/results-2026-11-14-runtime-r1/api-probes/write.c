#include <unistd.h>
void * volatile required_symbol = (void *)&write;
int main(void) { return required_symbol == 0; }
