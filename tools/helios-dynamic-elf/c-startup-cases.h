/* Shared deterministic mutations. All rejections must leave the result untouched. */
static const struct {
    const char *name;
    unsigned int word;
    unsigned long value;
    enum startup_error expected;
} startup_cases[] = {
    {"argc", 0, ~0UL, START_ARGV},
    {"argv-null", 3, 1, START_ARGV},
    {"argv-pointer", 1, ~0UL, START_BOUNDS},
    {"argv-unterminated", 0, 0, START_STRING},
    {"envp-null", 4, 1, START_ENV},
    {"auxv-tag", 5, 0, START_AUX},
    {"auxv-value", 12, 0, START_AUX},
    {"auxv-null", 30, 1, START_AUX},
    {"alignment", 0, 0, START_ALIGNMENT},
    {"overflow", 0, 0, START_BOUNDS},
    {"truncated", 0, 0, START_BOUNDS},
};

static int reject_startup_case(void *snapshot, unsigned long base, unsigned long size,
                               struct startup_image image, unsigned int index)
{
    unsigned long *words = snapshot;
    unsigned char *bytes = snapshot;
    char mode = '?';
    unsigned long argv = base + 8, envp = base + 32;
    unsigned int word = startup_cases[index].word;
    unsigned long original = words[word];
    unsigned long terminator = words[1] - base + sizeof(startup_name) - 1;
    switch (index) {
    case 3: bytes[terminator] = 'X'; break;
    case 8: base += 8; break;
    case 9: base = ~15UL; break;
    case 10: size = START_WORDS * 8 - 1; break;
    default: words[word] = startup_cases[index].value; break;
    }
    enum startup_error error = parse_startup(snapshot, base, size, 2, argv, envp, image, &mode);
    bytes[terminator] = 0;
    words[word] = original;
    return error == startup_cases[index].expected && mode == '?';
}
