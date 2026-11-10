#define _GNU_SOURCE
#include <dirent.h>
int main(void){DIR *d=opendir("."); if(!d) return 1; int bad=readdir(d)==0; closedir(d); return bad;}
