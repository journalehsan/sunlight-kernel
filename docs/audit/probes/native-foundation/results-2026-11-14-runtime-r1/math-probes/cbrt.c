extern double cbrt(double);
void * volatile requirement=(void *)&cbrt;
int main(void){return requirement==0;}
