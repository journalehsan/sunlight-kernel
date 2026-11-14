extern double acos(double);
void * volatile requirement=(void *)&acos;
int main(void){return requirement==0;}
