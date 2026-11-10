#define _GNU_SOURCE
#include <stdlib.h>
#include <unistd.h>
static void bye(void){write(1,"atexit OK\n",10);}
int main(void){return atexit(bye);}
