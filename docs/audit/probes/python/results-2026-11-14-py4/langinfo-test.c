/* Native C-locale metadata and lifetime/concurrent reader acceptance. */
#include <langinfo.h>
#include <locale.h>
#include <errno.h>
#include <string.h>
#include <stdio.h>
#include <pthread.h>
#include <stdint.h>
#include <limits.h>
_Static_assert(sizeof(nl_item)==sizeof(int),"native nl_item ABI");
static const nl_item days[]={DAY_1,DAY_2,DAY_3,DAY_4,DAY_5,DAY_6,DAY_7};
static const char *names[]={"Sunday","Monday","Tuesday","Wednesday","Thursday","Friday","Saturday"};
static const nl_item months[]={MON_1,MON_2,MON_3,MON_4,MON_5,MON_6,MON_7,MON_8,MON_9,MON_10,MON_11,MON_12};
static const char *month_names[]={"January","February","March","April","May","June","July","August","September","October","November","December"};
static const nl_item abdays[]={ABDAY_1,ABDAY_2,ABDAY_3,ABDAY_4,ABDAY_5,ABDAY_6,ABDAY_7};
static const nl_item abmonths[]={ABMON_1,ABMON_2,ABMON_3,ABMON_4,ABMON_5,ABMON_6,ABMON_7,ABMON_8,ABMON_9,ABMON_10,ABMON_11,ABMON_12};
static int verify(void){
 errno=71;
 for(unsigned i=0;i<7;i++) if(strcmp(nl_langinfo(days[i]),names[i])||strncmp(nl_langinfo(abdays[i]),names[i],3)||strlen(nl_langinfo(abdays[i]))!=3)return 1;
 for(unsigned i=0;i<12;i++) if(strcmp(nl_langinfo(months[i]),month_names[i])||strncmp(nl_langinfo(abmonths[i]),month_names[i],3)||strlen(nl_langinfo(abmonths[i]))!=3)return 2;
 if(strcmp(nl_langinfo(D_T_FMT),"%a %b %e %H:%M:%S %Y")||strcmp(nl_langinfo(D_FMT),"%m/%d/%y")||strcmp(nl_langinfo(T_FMT),"%H:%M:%S")||strcmp(nl_langinfo(T_FMT_AMPM),"%I:%M:%S %p"))return 3;
 if(strcmp(nl_langinfo(AM_STR),"AM")||strcmp(nl_langinfo(PM_STR),"PM")||strcmp(nl_langinfo(RADIXCHAR),".")||*nl_langinfo(THOUSEP)||*nl_langinfo(CRNCYSTR))return 4;
 const nl_item invalid[]={INT_MIN,-1,0,2,INT_MAX};
 for(unsigned i=0;i<sizeof(invalid)/sizeof(*invalid);i++)if(*nl_langinfo(invalid[i]))return 5;
 if(errno!=71)return 6;
 return 0;
}
static void *reader(void *arg){(void)arg;for(int i=0;i<1000;i++){int r=verify();if(r)return (void *)(uintptr_t)r;}return 0;}
int main(void){
 if(!setlocale(LC_ALL,"C")||verify())return 1;
 char *saved=nl_langinfo(DAY_1);
 for(int i=0;i<1000;i++)if(!setlocale(LC_CTYPE,i%2?"C":"C.UTF-8")||verify()||strcmp(nl_langinfo(CODESET),i%2?"US-ASCII":"UTF-8"))return 2;
 if(saved!=nl_langinfo(DAY_1)||strcmp(saved,"Sunday"))return 3;
 if(!setlocale(LC_ALL,"C"))return 4;
 pthread_t threads[4];
 for(int i=0;i<4;i++)if(pthread_create(&threads[i],0,reader,0))return 5;
 for(int i=0;i<4;i++){void *r=0;if(pthread_join(threads[i],&r)||r)return 6;}
 puts("PY4 C LOCALE METADATA PASS; calendar execution NOT_IMPLEMENTED");return 0;
}
