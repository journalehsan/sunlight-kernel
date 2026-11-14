#include <sys/stat.h>
int main(void) { struct stat metadata; return stat("/", &metadata); }
