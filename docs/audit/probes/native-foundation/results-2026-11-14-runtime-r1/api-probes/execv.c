#include <unistd.h>
void * volatile required_symbol = (void *)&execv;
int main(void) { return required_symbol == 0; }
