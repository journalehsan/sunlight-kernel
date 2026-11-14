extern double asin(double);
void * volatile requirement=(void *)&asin;
int main(void){return requirement==0;}
