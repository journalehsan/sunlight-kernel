extern double fabs(double);
void * volatile requirement=(void *)&fabs;
int main(void){return requirement==0;}
