#define _GNU_SOURCE
#include <termios.h>
#include <errno.h>
#include <unistd.h>
int main(void){struct termios t; int r=tcgetattr(0,&t); return r==0 || errno==ENOTTY ? 0 : 1;}
