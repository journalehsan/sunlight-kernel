#include <unistd.h>
void * volatile required_symbol = (void *)&pipe;
int main(void) { return required_symbol == 0; }
