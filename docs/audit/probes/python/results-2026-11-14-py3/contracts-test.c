#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <wchar.h>
#include <locale.h>
#include <limits.h>
#include <errno.h>
#include <time.h>
#include <pthread.h>
#define CHECK(x) do { if (!(x)) {printf("FAIL line=%d errno=%d\n",__LINE__,errno);return __LINE__;} }while(0)
static void *convert_worker(void *p){
 (void)p;
 for(int i=0;i<1000;i++){
  wchar_t w[4];char b[8];
  if(mbstowcs(w,"A\xc3\xa9",4)!=2||wcstombs(b,w,8)!=3||strcmp(b,"A\xc3\xa9"))return (void*)1;
  struct lconv *v=localeconv();if(strcmp(v->decimal_point,".")||v->frac_digits!=CHAR_MAX)return (void*)1;
 }
 return 0;
}
int main(void){
 unsigned char d[12];memset(d,0xa5,sizeof d);
 CHECK(strncpy((char*)d+1,"x",7)==(char*)d+1);CHECK(d[0]==0xa5&&d[1]=='x'&&d[8]==0xa5);for(int i=2;i<8;i++)CHECK(d[i]==0);
 char raw[3]={'a','b','c'};CHECK(strncpy((char*)d+1,raw,3)==(char*)d+1);CHECK(!memcmp(d+1,"abc",3));
 CHECK(strncpy((char*)d,raw,0)==(char*)d);
 CHECK(setlocale(LC_ALL,"C"));
 wchar_t w[8]={99,99,99,99,99,99,99,99};char b[12];memset(b,0x55,sizeof b);
 CHECK(mbstowcs(w,"ab",0)==0&&w[0]==99);
 CHECK(mbstowcs(w,"ab",2)==2&&w[2]==99);
 CHECK(mbstowcs(w,"ab",8)==2&&w[2]==0&&w[3]==99);
 CHECK(mbstowcs(0,"abcd",0)==4);
 errno=0;CHECK(mbstowcs(w,"\xc3\xa9",8)==(size_t)-1&&errno==EILSEQ);
 CHECK(wcstombs(b,L"abc",2)==2&&b[2]==0x55);
 CHECK(wcstombs(0,L"abc",0)==3);
 CHECK(setlocale(LC_ALL,"C.UTF-8"));
 CHECK(mbstowcs(w,"A\xc3\xa9\xf0\x9f\x98\x80",8)==3&&w[0]=='A'&&w[1]==0xe9&&w[2]==0x1f600&&w[3]==0);
 memset(b,0x55,sizeof b);CHECK(wcstombs(b,w,2)==1&&b[0]=='A'&&b[1]==0x55);
 CHECK(wcstombs(b,w,12)==7&&!strcmp(b,"A\xc3\xa9\xf0\x9f\x98\x80"));
 CHECK(wcstombs(0,w,0)==7);
 const char *bad[]={"\xc0\x80","\xed\xa0\x80","\xf4\x90\x80\x80","\xe2\x82","\x80"};
 for(unsigned i=0;i<sizeof bad/sizeof bad[0];i++){errno=0;CHECK(mbstowcs(w,bad[i],8)==(size_t)-1&&errno==EILSEQ);}
 wchar_t badw[]={0xd800,0};errno=0;CHECK(wcstombs(b,badw,12)==(size_t)-1&&errno==EILSEQ);badw[0]=-1;CHECK(wcstombs(b,badw,12)==(size_t)-1);
 struct lconv *lc=localeconv();CHECK(!strcmp(lc->decimal_point,".")&&!strcmp(lc->grouping,"")&&!strcmp(lc->currency_symbol,"")&&lc->int_n_sign_posn==CHAR_MAX);
 pthread_t t[4];for(int i=0;i<4;i++)CHECK(!pthread_create(t+i,0,convert_worker,0));for(int i=0;i<4;i++){void *r;CHECK(!pthread_join(t[i],&r)&&!r);}
 struct timespec ts={-1,-1};errno=77;CHECK(!clock_getres(CLOCK_REALTIME,&ts)&&ts.tv_sec==1&&ts.tv_nsec==0&&errno==77);
 CHECK(!clock_getres(CLOCK_MONOTONIC,&ts)&&ts.tv_sec==0&&ts.tv_nsec==10000000);CHECK(!clock_getres(CLOCK_REALTIME,0));
 ts.tv_sec=33;errno=0;CHECK(clock_getres(-1,&ts)==-1&&errno==EINVAL&&ts.tv_sec==33);
 FILE *f=tmpfile();CHECK(f);errno=77;CHECK(!setvbuf(f,0,_IONBF,0)&&errno==77);
 errno=0;CHECK(setvbuf(f,b,_IOFBF,sizeof b)!=0&&errno==ENOSYS);CHECK(setvbuf(f,b,_IOLBF,sizeof b)!=0&&errno==ENOSYS);
 CHECK(setvbuf(f,0,99,0)!=0&&errno==EINVAL);errno=77;setbuf(f,0);CHECK(errno==77);
 errno=0;setbuf(f,b);CHECK(errno==ENOSYS);CHECK(setvbuf(0,0,_IONBF,0)!=0&&errno==EINVAL);
 CHECK(fwrite("abc",1,3,f)==3&&!fseek(f,0,SEEK_SET));memset(b,0,sizeof b);CHECK(fread(b,1,3,f)==3&&!strcmp(b,"abc"));CHECK(!fclose(f));
 puts("PY3 CONTRACTS PASS bulk-conversion locale stream-policy resolution strncpy concurrency");return 0;
}
