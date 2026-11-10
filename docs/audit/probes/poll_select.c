#define _GNU_SOURCE
#include <poll.h>
#include <sys/select.h>
int main(void){struct timeval t={0,0}; return poll(0,0,0)!=0 || select(0,0,0,0,&t)!=0;}
