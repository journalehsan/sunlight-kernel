extern double trunc(double);
void * volatile requirement=(void *)&trunc;
int main(void){return requirement==0;}
