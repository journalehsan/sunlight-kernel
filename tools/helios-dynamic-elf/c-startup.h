#ifndef HELIOS_C_STARTUP_H
#define HELIOS_C_STARTUP_H

#ifdef HELIOS_C_DSO
#include "c-utility.h"
#define START_NAME HELIOS_C_DSO_PATH
#define START_BASE_TAG 7,
#define START_BASE_VALUE 0x80000000UL,
#define START_EXTRA_WORDS 2
#else
#define START_NAME "c-startup"
#define START_BASE_TAG
#define START_BASE_VALUE
#define START_EXTRA_WORDS 0
#endif

/* Exact shape of this fixture's two-argument, empty-environment entry. */
enum { START_WORDS = 31 + START_EXTRA_WORDS, START_RANDOM = 16 };
static const unsigned long startup_tags[] = {3, 4, 5, 6, 9, 11, 12, 13, 14, 23, START_BASE_TAG 31, 25, 0};
static const char startup_name[] = START_NAME;

struct startup_image {
    unsigned long phdr, phnum, entry;
};

enum startup_error {
    START_OK, START_ALIGNMENT, START_BOUNDS, START_ARGV, START_STRING,
    START_ENV, START_AUX
};

/* Resolve an encoded guest address only through the bounded stack snapshot. */
static int stack_contains(unsigned long base, unsigned long size, unsigned long pointer,
                          unsigned long length)
{
    return pointer >= base && pointer - base <= size && length <= size - (pointer - base);
}

static enum startup_error parse_startup(const void *snapshot, unsigned long base,
                                         unsigned long size, unsigned long argc,
                                         unsigned long argv, unsigned long envp,
                                         struct startup_image image, char *mode)
{
    const unsigned long table_bytes = START_WORDS * sizeof(unsigned long);
    if (base & 15)
        return START_ALIGNMENT;
    if (size < table_bytes || size > ~0UL - base)
        return START_BOUNDS;
    const unsigned long *words = snapshot;
    const unsigned long data = base + table_bytes;
    const unsigned long data_size = size - table_bytes;
    if (words[0] != 2 || argc != 2 || argv != base + 8 || words[3])
        return START_ARGV;
    if (envp != base + 32 || words[4])
        return START_ENV;
    const unsigned long random = base + size - START_RANDOM;
    if (!stack_contains(data, data_size, words[1], sizeof(startup_name)) ||
        !stack_contains(data, data_size, words[2], 2) ||
        !stack_contains(data, data_size, random, START_RANDOM) ||
        words[1] + sizeof(startup_name) != random || words[2] + 2 != words[1])
        return START_BOUNDS;

    const unsigned char *bytes = snapshot;
    for (unsigned int i = 0; i < sizeof(startup_name); ++i)
        if (bytes[words[1] - base + i] != (unsigned char)startup_name[i])
            return START_STRING;
    const char value = bytes[words[2] - base];
    if ((value != 'P' && value != 'L' && value != 'N' && value != 'A' && value != 'W') ||
        bytes[words[2] - base + 1])
        return START_STRING;

    const unsigned long aux[] = {image.phdr, 56, image.phnum, 4096, image.entry,
                                 words[16], words[16], words[20], words[20],
                                 0, START_BASE_VALUE words[1], random, 0};
    for (unsigned int i = 0; i < sizeof(startup_tags) / sizeof(startup_tags[0]); ++i)
        if (words[5 + 2*i] != startup_tags[i] || words[6 + 2*i] != aux[i])
            return START_AUX;
    *mode = value;
    return START_OK;
}

#endif
