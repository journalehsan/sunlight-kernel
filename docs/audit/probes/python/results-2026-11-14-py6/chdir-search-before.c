#include <stdio.h>
int main(void){long r;const char*p="/tmp/native-c-denied";__asm__ volatile("syscall":"=a"(r):"a"(63),"D"(p):"rcx","r11","memory");printf("chdir denied directory returned %ld, expected -4\n",r);return r==-4?0:1;}
