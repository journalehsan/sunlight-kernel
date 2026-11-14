extern double asinh(double);
void * volatile requirement=(void *)&asinh;
int main(void){return requirement==0;}
