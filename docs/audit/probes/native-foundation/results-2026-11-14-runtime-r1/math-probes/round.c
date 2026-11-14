extern double round(double);
void * volatile requirement=(void *)&round;
int main(void){return requirement==0;}
