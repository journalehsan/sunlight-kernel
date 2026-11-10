/* Negative runner control: an actual user #UD must fail even if its exit
 * status matches --expect-exit 132, and the normal reaper must still run. */
int main(void) {
    __asm__ volatile ("ud2");
    return 0;
}
