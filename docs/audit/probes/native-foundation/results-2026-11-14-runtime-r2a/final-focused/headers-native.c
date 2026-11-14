#include <sys/types.h>
#include <sys/mman.h>
#include <unistd.h>
#include <time.h>
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <inttypes.h>
#include <errno.h>
#include <assert.h>
#ifdef __cplusplus
#define SA static_assert
#define ALIGN alignof
#else
#define SA _Static_assert
#define ALIGN _Alignof
#endif
#define ABI(t,n,a) SA(sizeof(t)==n && ALIGN(t)==a,#t " ABI")
ABI(ssize_t,8,8); ABI(off_t,8,8); ABI(pid_t,4,4); ABI(mode_t,4,4);
ABI(dev_t,8,8); ABI(ino_t,8,8); ABI(nlink_t,4,4); ABI(uid_t,4,4); ABI(gid_t,4,4);
ABI(time_t,8,8); ABI(clock_t,8,8); ABI(intmax_t,8,8); ABI(uintmax_t,8,8);
ABI(long double,16,16);
SA(__LDBL_MANT_DIG__==64,"x87 extended, not double");
SA((ssize_t)-1<0 && (off_t)-1<0 && (pid_t)-1<0,"signed types");
SA((mode_t)-1>0 && (uid_t)-1>0 && (ino_t)-1>0,"unsigned types");
static int disabled(void) {
 int x=0;
#define NDEBUG
#include <assert.h>
 assert(++x);
#undef NDEBUG
#include <assert.h>
 return x;
}
static void callback(void) { puts("UNEXPECTED assertion callback"); }
int main(int argc, char **argv) {
 (void)argv;
 if(argc>1) { atexit(callback); assert(0 && "R2A diagnostic"); puts("UNEXPECTED assertion return");return 1; }
 int x=0;assert(++x);assert(x==1);assert(disabled()==0);
 imaxdiv_t d=imaxdiv(-17,5);assert(d.quot==-3 && d.rem==-2);assert(imaxabs(-42)==42);
 char *end;const char *s="  -0x8000000000000000!";
 errno=0;assert(strtoimax(s,&end,0)==INTMAX_MIN && *end=='!' && errno==0);
 errno=0;assert(strtoimax("9223372036854775808",&end,10)==INTMAX_MAX && errno==ERANGE && !*end);
 errno=0;assert(strtoimax("-9223372036854775809",&end,10)==INTMAX_MIN && errno==ERANGE);
 errno=0;assert(strtoumax("18446744073709551615!",&end,10)==UINTMAX_MAX && *end=='!' && errno==0);
 errno=0;assert(strtoumax("18446744073709551616xyz",&end,10)==UINTMAX_MAX && errno==ERANGE && *end=='x');
 assert(strtoumax("-1",&end,10)==UINTMAX_MAX);assert(strtoimax("077",&end,0)==63);
 assert(strtoimax("0x",&end,0)==0 && *end=='x');assert(strtoimax("z",&end,36)==35);
 s=" +!";assert(strtoimax(s,&end,10)==0 && end==s);
 errno=0;s="12";assert(strtoimax(s,&end,1)==0 && end==s && errno==EINVAL);
 assert(strtoimax("123",NULL,10)==123);
 wchar_t *wend;assert(wcstoimax(L"-123!",&wend,10)==-123 && *wend==L'!');
 assert(wcstoumax(L"ffffffffffffffff",&wend,16)==UINTMAX_MAX && !*wend);
 printf("formats %" PRId8 " %" PRIu16 " %" PRId32 " %" PRIu64 " %" PRIdMAX " %" PRIxPTR "\n",(int8_t)-8,(uint16_t)16,(int32_t)-32,(uint64_t)64,(intmax_t)-99,(uintptr_t)0xabc);
 puts("R2A headers PASS");return 0;
}
