extern double tanh(double);
void * volatile requirement=(void *)&tanh;
int main(void){return requirement==0;}
