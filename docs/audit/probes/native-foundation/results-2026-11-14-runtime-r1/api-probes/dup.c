#include <unistd.h>
void * volatile required_symbol = (void *)&dup;
int main(void) { return required_symbol == 0; }
