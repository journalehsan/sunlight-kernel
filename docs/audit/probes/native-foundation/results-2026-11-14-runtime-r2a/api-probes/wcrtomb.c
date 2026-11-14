#include <wchar.h>
void * volatile required_symbol = (void *)&wcrtomb;
int main(void) { return required_symbol == 0; }
