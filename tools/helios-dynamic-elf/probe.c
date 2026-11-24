#include <unistd.h>

int main(void)
{
    static const char message[] = "[DYNAMIC-ELF] probe write OK\n";
    return write(STDOUT_FILENO, message, sizeof(message) - 1)
        == sizeof(message) - 1 ? 0 : 1;
}
