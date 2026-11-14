extern double atan(double);
void * volatile requirement=(void *)&atan;
int main(void){return requirement==0;}
