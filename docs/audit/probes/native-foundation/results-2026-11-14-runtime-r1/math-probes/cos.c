extern double cos(double);
void * volatile requirement=(void *)&cos;
int main(void){return requirement==0;}
