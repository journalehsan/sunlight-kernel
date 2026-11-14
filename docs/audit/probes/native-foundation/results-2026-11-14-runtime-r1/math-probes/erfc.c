extern double erfc(double);
void * volatile requirement=(void *)&erfc;
int main(void){return requirement==0;}
