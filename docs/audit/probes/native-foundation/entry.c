/* Canonical _start calls main, then passes its return value to native exit. */
int main(int argc, char **argv) {
    return argc > 0 && argv && argv[0] && argv[argc] == 0 ? 0 : 11;
}
