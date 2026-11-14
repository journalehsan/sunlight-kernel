extern double atanh(double);
void * volatile requirement=(void *)&atanh;
int main(void){return requirement==0;}
