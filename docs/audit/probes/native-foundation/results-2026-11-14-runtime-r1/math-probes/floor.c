extern double floor(double);
void * volatile requirement=(void *)&floor;
int main(void){return requirement==0;}
