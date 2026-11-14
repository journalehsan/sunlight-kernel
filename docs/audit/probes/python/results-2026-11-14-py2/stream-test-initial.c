#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <errno.h>
#include <pthread.h>
#define CHECK(x) do {if(!(x)){printf("STREAM FAIL line=%d errno=%d\n",__LINE__,errno);return 1;}}while(0)
static FILE *shared;
static void *writer(void *p) {long id=(long)p;for(int i=0;i<50;i++) if(fprintf(shared,"%ld:%03d\n",id,i)!=6) return (void*)1;return 0;}
int main(void) {
 FILE *f=tmpfile();CHECK(f);CHECK(ftell(f)==0);CHECK(fgetc(f)==EOF && feof(f) && !ferror(f));
 CHECK(ungetc('X',f)=='X' && !feof(f));CHECK(fgetc(f)=='X');CHECK(fgetc(f)==EOF && feof(f));
 clearerr(f);CHECK(!feof(f) && !ferror(f));
 CHECK(fseek(f,0,SEEK_SET)==0);CHECK(fwrite("abcdef",2,3,f)==3);CHECK(fflush(f)==0 && ftell(f)==6);
 rewind(f);CHECK(fgetc(f)=='a');CHECK(ungetc('Z',f)=='Z' && ftell(f)==0);CHECK(ungetc('Y',f)==EOF);CHECK(fgetc(f)=='Z');
 CHECK(ungetc('Q',f)=='Q');CHECK(fseek(f,0,SEEK_CUR)==0 && fgetc(f)=='a');
 CHECK(fseek(f,-2,SEEK_END)==0 && ftell(f)==4);
 char b[2048];CHECK(fread(b,2,2,f)==1 && !memcmp(b,"ef",2) && feof(f) && !ferror(f));
 CHECK(fseek(f,0,99)==-1 && ferror(f));clearerr(f);CHECK(!ferror(f));
 CHECK(fread(0,0,99,f)==0 && fwrite(0,99,0,f)==0);
 int fd=fileno(f);CHECK(fd>=0);CHECK(fclose(f)==0);CHECK(fcntl(fd,F_GETFD)==-1 && errno==EBADF);
 errno=0;CHECK(fdopen(-1,"r")==0 && errno==EBADF);
 f=fopen("/tmp/py2-stream","w+");CHECK(f);CHECK(fwrite("retained",1,8,f)==8);CHECK(fclose(f)==0);
 fd=open("/tmp/py2-stream",O_RDWR);CHECK(fd>=0);CHECK(fdopen(fd,"invalid")==0 && fcntl(fd,F_GETFD)>=0);
 f=fdopen(fd,"w+");CHECK(f);CHECK(fread(b,1,8,f)==8 && !memcmp(b,"retained",8));CHECK(fclose(f)==0);
 fd=open("/tmp/py2-stream",O_WRONLY);CHECK(fd>=0);CHECK(fdopen(fd,"r")==0 && errno==EINVAL);CHECK(fcntl(fd,F_GETFD)>=0);CHECK(close(fd)==0);
 fd=open("/tmp/py2-stream",O_RDWR);CHECK(fd>=0);f=fdopen(fd,"a+");CHECK(f);CHECK(fseek(f,0,SEEK_SET)==0);CHECK(fwrite("!",1,1,f)==1);rewind(f);CHECK(fread(b,1,9,f)==9 && !memcmp(b,"retained!",9));CHECK(fclose(f)==0);
 f=fopen("/tmp/py2-stream","r");CHECK(f);CHECK(fwrite("x",1,1,f)==0 && ferror(f));CHECK(fclose(f)==0);
 f=tmpfile();CHECK(f);memset(b,'z',sizeof b);CHECK(fwrite(b,1,sizeof b,f)==sizeof b);rewind(f);memset(b,0,sizeof b);CHECK(fread(b,1,sizeof b,f)==sizeof b);for(unsigned i=0;i<sizeof b;i++) CHECK(b[i]=='z');CHECK(!feof(f));CHECK(fgetc(f)==EOF && feof(f));CHECK(fclose(f)==0);
 shared=tmpfile();CHECK(shared);pthread_t t[4];for(long i=0;i<4;i++) CHECK(pthread_create(&t[i],0,writer,(void*)i)==0);for(int i=0;i<4;i++){void *r=(void*)1;CHECK(pthread_join(t[i],&r)==0 && !r);}
 CHECK(ftell(shared)==1200);rewind(shared);int lines=0;while(fgets(b,sizeof b,shared)){CHECK(strlen(b)==6 && b[1]==':' && b[5]=='\n');lines++;}CHECK(lines==200 && feof(shared) && !ferror(shared));CHECK(fclose(shared)==0);
 puts("PY2 STREAM PASS checks=ownership,append,eof,error,seek,pushback,concurrency");return 0;
}
