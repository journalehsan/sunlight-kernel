extern double frexp(double);
void * volatile requirement=(void *)&frexp;
int main(void){return requirement==0;}
