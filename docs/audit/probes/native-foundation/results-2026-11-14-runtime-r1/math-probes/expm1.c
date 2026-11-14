extern double expm1(double);
void * volatile requirement=(void *)&expm1;
int main(void){return requirement==0;}
