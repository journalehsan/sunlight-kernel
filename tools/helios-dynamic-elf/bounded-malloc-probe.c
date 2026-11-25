static void malloc_trace(const struct page_allocator *pages, struct page_handle handle,
                         unsigned long size, long status)
{
    char line[160] = "[BOUNDED-MALLOC] ";
    char *cursor = line + sizeof("[BOUNDED-MALLOC] ") - 1;
    append_hex(&cursor, pages->owner);
    append_hex(&cursor, handle.slot);
    append_hex(&cursor, handle.generation);
    append_hex(&cursor, size);
    append_hex(&cursor, HELIOS_MALLOC_ALIGNMENT);
    append_hex(&cursor, (unsigned long)status);
    cursor[-1] = '\n';
    if (call(1, 1, (unsigned long)line, cursor - line, 0, 0) != cursor - line)
        allocator_fatal();
}

static int malloc_contract(void)
{
    struct helios_heap heap = {.pages.owner = (unsigned long)call(39, 0, 0, 0, 0, 0)};
    struct helios_allocation live, unpublished = {0};
    const unsigned long size = PAGE_BYTES + 19;
    if (helios_malloc(&heap, &guest_pages, size, 0, &live))
        return 1;
    malloc_trace(&heap.pages, live.backing, size, 0);
    volatile unsigned char *bytes = (volatile unsigned char *)helios_resolve(&heap, live);
    if (!bytes || (unsigned long)bytes % HELIOS_MALLOC_ALIGNMENT ||
        !bytes_match(&heap.pages, live.backing, 0, 0x71))
        return 1;
    struct helios_allocation invalid = live;
    ++invalid.backing.base;
    if (helios_free(&heap, &guest_pages, invalid) != PAGE_INVALID ||
        !LOG("[MALLOC-CHECK] misaligned=-22\n"))
        return 1;
    invalid = live;
    ++invalid.backing.generation;
    if (helios_free(&heap, &guest_pages, invalid) != PAGE_INVALID ||
        !LOG("[MALLOC-CHECK] generation=-22\n"))
        return 1;
    invalid = live;
    ++invalid.requested;
    if (helios_free(&heap, &guest_pages, invalid) != PAGE_INVALID ||
        !LOG("[MALLOC-CHECK] size=-22\n"))
        return 1;
    long status = helios_malloc(&heap, &guest_pages, 19, reject_publication, &unpublished);
    malloc_trace(&heap.pages, unpublished.backing, 19, status);
    if (status != PAGE_PUBLICATION || unpublished.requested ||
        helios_free(&heap, &guest_pages, unpublished) != PAGE_INVALID ||
        heap.slots[1].requested || heap.pages.used != 1 || heap.pages.pages != 2 ||
        heap.pages.generation != live.backing.generation ||
        !bytes_match(&heap.pages, live.backing, 0x71, 0) ||
        !LOG("[MALLOC-CHECK] shim-publication=-5 unpublished=-22 owners=1 pages=2 marker=71\n"))
        return 1;

    if (helios_free(&heap, &guest_pages, live) || heap.pages.used || heap.pages.pages ||
        heap.slots[live.backing.slot].requested)
        return 1;
    return !LOG("[MALLOC-CHECK] done owners=0 pages=0\n");
}

static int malloc_setup_failure(void)
{
    struct helios_heap heap = {.pages.owner = (unsigned long)call(39, 0, 0, 0, 0, 0)};
    struct helios_allocation live;
    if (helios_malloc(&heap, &guest_pages, PAGE_BYTES + 1, 0, &live) ||
        !bytes_match(&heap.pages, live.backing, 0, 0x4a))
        return 1;
    malloc_trace(&heap.pages, live.backing, live.requested, 0);
    return LOG("[MALLOC-CHECK] child-setup-aborted live=2 marker=4a exit=43\n") ? 43 : 1;
}

static int malloc_foreign_probe(const char *token)
{
    struct helios_heap heap = {.pages.owner = (unsigned long)call(39, 0, 0, 0, 0, 0)};
    struct helios_allocation live;
    if (helios_malloc(&heap, &guest_pages, 37, 0, &live) ||
        !bytes_match(&heap.pages, live.backing, 0, token ? 0x59 : 0x58))
        return 1;
    if (!token) {
        unsigned long words[] = {live.backing.owner, live.backing.generation,
                                 live.backing.base, live.backing.pages,
                                 live.backing.slot, live.requested};
        if (call(1, 1, (unsigned long)words, sizeof(words), 0, 0) != sizeof(words))
            return 1;
    } else {
        malloc_trace(&heap.pages, live.backing, live.requested, 0);
        unsigned long words[6] = {0};
        for (unsigned int i = 0; i < 6; ++i) {
            for (unsigned int digit = 0; digit < 16; ++digit) {
                unsigned char ch = (unsigned char)*token++;
                if (ch >= '0' && ch <= '9')
                    ch -= '0';
                else if (ch >= 'a' && ch <= 'f')
                    ch -= 'a' - 10;
                else
                    return 1;
                words[i] = words[i] * 16 + ch;
            }
        }
        const struct helios_allocation foreign = {
            {words[0], words[1], words[2], words[3], words[4]}, words[5]
        };
        if (*token || foreign.backing.owner == heap.pages.owner ||
            foreign.backing.base != live.backing.base ||
            foreign.backing.generation != live.backing.generation ||
            foreign.backing.pages != live.backing.pages ||
            foreign.backing.slot != live.backing.slot || foreign.requested != live.requested ||
            helios_free(&heap, &guest_pages, foreign) != PAGE_INVALID ||
            heap.pages.used != 1 || heap.pages.pages != 1 ||
            !bytes_match(&heap.pages, live.backing, 0x59, 0))
            return 1;
        allocator_trace(1, 'F', &heap.pages, foreign.backing, 0x59, PAGE_INVALID);
        if (!LOG("[MALLOC-CHECK] foreign-child-token=-22 owners=1 pages=1 marker=59\n"))
            return 1;
    }
    return helios_free(&heap, &guest_pages, live) != 0;
}
