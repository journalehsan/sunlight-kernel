extern double sin(double);
void * volatile requirement=(void *)&sin;
int main(void){return requirement==0;}
