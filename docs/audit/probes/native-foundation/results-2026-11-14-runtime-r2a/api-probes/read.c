#include <unistd.h>
void * volatile required_symbol = (void *)&read;
int main(void) { return required_symbol == 0; }
