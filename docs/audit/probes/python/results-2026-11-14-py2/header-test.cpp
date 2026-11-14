#include <stdio.h>
#include <fcntl.h>
#include <wchar.h>
#include <stdlib.h>
#include <string.h>
extern "C" int main() {char b[64];wchar_t w[8];if(snprintf(b,sizeof b,"%.2f:%ld",1.25,7L)!=6 || strcmp(b,"1.25:7"))return 1;if(wcslen(wcscpy(w,L"test"))!=4)return 2;return 0;}
