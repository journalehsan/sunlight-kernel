extern double acosh(double);
void * volatile requirement=(void *)&acosh;
int main(void){return requirement==0;}
