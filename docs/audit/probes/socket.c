#define _GNU_SOURCE
#include <sys/socket.h>
#include <unistd.h>
int main(void){int f=socket(AF_INET,SOCK_STREAM,0); if(f<0)return 1; return close(f);}
