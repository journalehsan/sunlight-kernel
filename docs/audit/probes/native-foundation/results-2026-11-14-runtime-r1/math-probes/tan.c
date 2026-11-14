extern double tan(double);
void * volatile requirement=(void *)&tan;
int main(void){return requirement==0;}
