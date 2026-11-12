#include <wchar.h>
#include <stdlib.h>
#include <limits.h>
#include <errno.h>
#include <locale.h>
#include <pthread.h>
#define CHECK(x) do { if (!(x)) return __LINE__; } while (0)
/* Source is ASCII: UTF-8 bytes are explicit, independent of source encoding. */
static int check(void) {
    const char *seq[] = {"\xc3\xa9", "\xd9\x81", "\xe2\x82\xac", "\xf0\x9f\x98\x80", "\xef\xbf\xbf"};
    int lens[] = {2,2,3,4,3};
    int values[] = {0xe9,0x641,0x20ac,0x1f600,0xffff};
    for (int j=0;j<5;j++) for (int k=1;k<lens[j];k++) {
        mbstate_t st={0}; wchar_t w=42;
        CHECK(mbsinit(&st));
        CHECK(mbrtowc(&w,seq[j],k,&st)==(size_t)-2 && w==42 && !mbsinit(&st));
        CHECK(mbrtowc(&w,seq[j]+k,lens[j]-k,&st)==(size_t)(lens[j]-k));
        CHECK(w==values[j] && mbsinit(&st));
    }
    for (int j=0;j<5;j++) {
        mbstate_t st={0}; wchar_t w=0;
        for(int k=0;k<lens[j];k++) CHECK(mbrtowc(&w,seq[j]+k,1,&st)==(k==lens[j]-1 ? 1 : (size_t)-2));
        CHECK(w==values[j]);
    }
    const char *bad[]={"\x80","\xc0\x80","\xe0\x80","\xed\xa0","\xf0\x80","\xf4\x90","\xf5","\xff","\xc2\x41"};
    for (int j=0;j<9;j++) { mbstate_t st={0}; errno=0; CHECK(mbrtowc(0,bad[j],2,&st)==(size_t)-1 && errno==EILSEQ); }
    mbstate_t st={0}; wchar_t w=42;
    CHECK(mbrtowc(&w,"",1,&st)==0 && w==0 && mbsinit(&st));
    CHECK(mbrtowc(&w,"A",0,&st)==(size_t)-2 && mbsinit(&st));
    CHECK(mbrtowc(&w,"A",1,&st)==1 && w==L'A');
    CHECK(mbrtowc(&w,0,0,&st)==0 && w==L'A');
    CHECK(mbrlen("\xc3",1,&st)==(size_t)-2);
    CHECK(mbrlen(0,0,&st)==(size_t)-1 && errno==EILSEQ);
    CHECK(mbsinit(0));
    return 0;
}
static void *worker(void *arg) { (void)arg; for(int i=0;i<100;i++) { int r=check(); if(r) return (void *)(size_t)r; } return 0; }
int main(void) {
    CHECK(setlocale(LC_ALL,"C") && MB_CUR_MAX==1 && MB_LEN_MAX==4);
    mbstate_t st={0}; wchar_t w=42;
    CHECK(mbrtowc(&w,"\xc3\xa9",2,&st)==(size_t)-1 && errno==EILSEQ);
    CHECK(mbrtowc(&w,"A",1,0)==1 && w==L'A');
    const char *names[]={"C.UTF-8","en_US.UTF-8"};
    for(int j=0;j<2;j++) {
        CHECK(setlocale(LC_CTYPE,names[j]) && MB_CUR_MAX==4);
        CHECK(check()==0);
        pthread_t a,b; void *ra,*rb;
        CHECK(pthread_create(&a,0,worker,0)==0 && pthread_create(&b,0,worker,0)==0);
        CHECK(pthread_join(a,&ra)==0 && pthread_join(b,&rb)==0 && !ra && !rb);
    }
    CHECK(setlocale(LC_ALL,"POSIX") && MB_CUR_MAX==1);
    return 0;
}
