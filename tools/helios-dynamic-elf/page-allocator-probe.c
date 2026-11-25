/* Included by anon-mmap.c in the freestanding allocator regression image. */
#include "page-allocator.h"
#ifdef MALLOC_FIXTURE
#include "bounded-malloc.h"
#endif

static long guest_page_map(unsigned long bytes)
{
    return call(MAP, 0, bytes, RW, 0x22, -1);
}

static long guest_page_unmap(unsigned long base, unsigned long bytes)
{
    return call(UNMAP, base, bytes, 0, 0, 0);
}

static void __attribute__((noreturn)) allocator_fatal(void)
{
    call(60, 1, 0, 0, 0, 0);
    __builtin_unreachable();
}

static int reject_publication(unsigned long base, unsigned long bytes)
{
    volatile unsigned char *page = (volatile unsigned char *)base;
    for (unsigned long i = 0; i < bytes; i += PAGE_BYTES)
        page[i] = 0x51;
    return 0;
}

static const struct page_backend guest_pages = {
    guest_page_map, guest_page_unmap, 0, allocator_fatal
};

static void append_hex(char **cursor, unsigned long value)
{
    for (int shift = 60; shift >= 0; shift -= 4)
        *(*cursor)++ = "0123456789abcdef"[(value >> shift) & 15];
    *(*cursor)++ = ' ';
}

static void allocator_trace(int malloc_mode, char op, const struct page_allocator *allocator,
                            struct page_handle handle, unsigned long marker, long status)
{
    (void)malloc_mode;
    char line[192] = "[PAGE-ALLOC] ";
    char *cursor = line + sizeof("[PAGE-ALLOC] ") - 1;
#ifdef MALLOC_FIXTURE
    if (malloc_mode)
        for (unsigned int i = 0; i < 10; ++i)
            line[i + 1] = "MALL-ALLOC"[i];
#endif
    *cursor++ = op;
    *cursor++ = ' ';
    append_hex(&cursor, allocator->owner);
    append_hex(&cursor, handle.owner);
    append_hex(&cursor, handle.slot);
    append_hex(&cursor, handle.generation);
    append_hex(&cursor, handle.base);
    append_hex(&cursor, handle.pages);
    append_hex(&cursor, allocator->used);
    append_hex(&cursor, allocator->pages);
    append_hex(&cursor, marker);
    append_hex(&cursor, (unsigned long)status);
    cursor[-1] = '\n';
    unsigned long length = (unsigned long)(cursor - line);
    if (call(1, 1, (unsigned long)line, length, 0, 0) != (long)length)
        allocator_fatal();
}

#define CHECK(condition) do { if (!(condition)) { \
    allocator_trace(malloc_mode, '!', &allocator, (struct page_handle){0}, __LINE__, 1); \
    return 1; \
} } while (0)

static int bytes_match(const struct page_allocator *allocator, struct page_handle handle,
                       unsigned char value, int mark)
{
    volatile unsigned char *bytes = (volatile unsigned char *)page_resolve(allocator, handle);
    if (!bytes)
        return 0;
    for (unsigned long i = 0; i < handle.pages * PAGE_BYTES; ++i) {
        if (bytes[i] != value)
            return 0;
        if (mark)
            bytes[i] = (unsigned char)mark;
    }
    return 1;
}

#ifdef MALLOC_FIXTURE
#include "bounded-malloc-probe.c"
#endif

static long attempt(int malloc_mode, struct page_allocator *allocator, const struct page_backend *backend,
                    unsigned long pages, unsigned char marker, struct page_handle *handle)
{
    long result;
#ifdef MALLOC_FIXTURE
    if (malloc_mode) {
        struct helios_heap *heap = (struct helios_heap *)allocator;
        struct helios_allocation allocation;
        unsigned long size = pages * PAGE_BYTES;
        if (pages <= PAGE_SINGLE_MAX && size)
            size -= PAGE_BYTES - 1 - marker;
        else if (pages == ~0UL)
            size = ~0UL;
        result = helios_malloc(heap, backend, size, 0, &allocation);
        if (!result)
            *handle = allocation.backing;
        malloc_trace(allocator, result ? (struct page_handle){0} : *handle, size, result);
    } else
#endif
        result = page_alloc(allocator, backend, pages, handle);
    struct page_handle diagnostic = result ? (struct page_handle){.pages = pages} : *handle;
    allocator_trace(malloc_mode, 'A', allocator, diagnostic, marker, result);
    if (!result && !bytes_match(allocator, *handle, 0, marker))
        allocator_fatal();
    return result;
}

static long release(int malloc_mode, struct page_allocator *allocator, struct page_handle handle,
                    unsigned char marker)
{
    long result;
#ifdef MALLOC_FIXTURE
    if (malloc_mode) {
        struct helios_heap *heap = (struct helios_heap *)allocator;
        unsigned long size = handle.slot < PAGE_SLOTS ? heap->slots[handle.slot].requested : 0;
        result = helios_free(heap, &guest_pages, (struct helios_allocation){handle, size});
    } else
#endif
        result = page_free(allocator, &guest_pages, handle);
    allocator_trace(malloc_mode, 'F', allocator, handle, marker, result);
    return result;
}

static int allocator_probe(unsigned char marker, int malloc_mode)
{
#ifdef MALLOC_FIXTURE
    struct helios_heap heap = {0};
#define allocator heap.pages
#else
    struct page_allocator allocator = {0};
#endif
    allocator.owner = (unsigned long)call(39, 0, 0, 0, 0, 0);
    struct page_handle handles[PAGE_SLOTS], replacement = {0};
    const int live_exit = marker >= 'a' && marker <= 'g';
    CHECK(allocator.owner && (long)allocator.owner > 0);
    allocator_trace(malloc_mode, 'I', &allocator, (struct page_handle){0}, marker, 0);

    if (live_exit)
        marker = (unsigned char)(0x80 + 3 * (marker - 'a'));
    CHECK(attempt(malloc_mode, &allocator, &guest_pages, 1, marker, &handles[0]) == 0);
    if (marker == 'T') {
        /* Keep the live sentinel in its existing leaf table. Advance the mmap
         * cursor to the next leaf table without populating the padding. */
        const unsigned long table_span = PAGE_BYTES * 512UL;
        unsigned long padding = (0UL - handles[0].base - PAGE_BYTES) & (table_span - 1);
        if (padding)
            CHECK(guest_page_map(padding) == (long)(handles[0].base + PAGE_BYTES));
    }
    if (marker == 'F' || marker == 'T' || marker == 'L' || marker == 'Q') {
        struct page_backend backend = guest_pages;
        if (marker == 'Q')
            backend.publish = reject_publication;
        unsigned long pages = marker == 'Q' ? 2 : 3;
        long expected = marker == 'Q' ? PAGE_PUBLICATION :
                        marker == 'L' ? PAGE_INVALID : PAGE_NO_MEMORY;
        CHECK(attempt(malloc_mode, &allocator, &backend, pages, marker, &replacement) == expected);
        CHECK(!replacement.generation && !page_resolve(&allocator, replacement));
        CHECK(release(malloc_mode, &allocator, replacement, marker) == PAGE_INVALID);
        CHECK(allocator.generation == 1 && allocator.used == 1 && allocator.pages == 1);
        CHECK(bytes_match(&allocator, handles[0], marker, 0));
        if (marker == 'T') {
            unsigned long padding = (0UL - handles[0].base - PAGE_BYTES) & ((PAGE_BYTES * 512UL) - 1);
            if (padding)
                CHECK(guest_page_unmap(handles[0].base + PAGE_BYTES, padding) == 0);
        }
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, 2, marker + 1, &replacement) == 0);
        CHECK(replacement.generation == 2 && replacement.slot == 1);
        CHECK(bytes_match(&allocator, replacement, marker + 1, 0));
        CHECK(release(malloc_mode, &allocator, replacement, marker) == 0);
    } else if (marker == 'E') {
        /* Adjacent RW allocator mappings share a ledger region. With the
         * ledger full, removing the middle mapping needs an extra entry. */
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, 1, marker + 1, &handles[1]) == 0);
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, 1, marker + 2, &handles[2]) == 0);
        CHECK(handles[1].base == handles[0].base + PAGE_BYTES);
        CHECK(handles[2].base == handles[1].base + PAGE_BYTES);
        long last = 0;
        unsigned int count = 0;
        for (; count < 256; ++count) {
            long base = call(MAP, 0, PAGE_BYTES, count % 2 ? RW : READ, 0x22, -1);
            if (base == PAGE_NO_MEMORY)
                break;
            CHECK(base > 0);
            last = base;
        }
        CHECK(count && count < 256);
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, 1, marker, &replacement) == PAGE_NO_MEMORY);
        CHECK(!replacement.generation && allocator.generation == 3);
        CHECK(release(malloc_mode, &allocator, handles[1], marker) == PAGE_NO_MEMORY);
        CHECK(allocator.used == 3 && allocator.pages == 3);
        for (unsigned int i = 0; i < 3; ++i)
            CHECK(bytes_match(&allocator, handles[i], marker + i, 0));
        CHECK(guest_page_unmap((unsigned long)last, PAGE_BYTES) == 0);
        CHECK(release(malloc_mode, &allocator, handles[1], marker) == 0);
        CHECK(release(malloc_mode, &allocator, handles[2], marker) == 0);
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, 1, marker + 3, &replacement) == 0);
        CHECK(replacement.slot == handles[1].slot && replacement.generation == 4);
        CHECK(bytes_match(&allocator, replacement, marker + 3, 0));
        CHECK(release(malloc_mode, &allocator, replacement, marker) == 0);
    } else {
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, 2, marker + 1, &handles[1]) == 0);
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, 3, marker + 2, &handles[2]) == 0);
        for (unsigned int i = 0; i < 3; ++i) {
            CHECK(bytes_match(&allocator, handles[i], marker + i, 0));
            for (unsigned int j = i + 1; j < 3; ++j)
                CHECK(handles[i].base + handles[i].pages * PAGE_BYTES <= handles[j].base ||
                      handles[j].base + handles[j].pages * PAGE_BYTES <= handles[i].base);
        }
        if (live_exit) {
            CHECK(allocator.used == 3 && allocator.pages == 6 && allocator.generation == 3);
#ifdef MALLOC_FIXTURE
            if (malloc_mode)
                return !LOG("[MALL-ALLOC-LIVE] zero=all markers=private allocations=3 pages=6 exit=live\n");
#endif
            return !LOG("[PAGE-ALLOC-LIVE] zero=all markers=private allocations=3 pages=6 exit=live\n");
        }

        struct page_handle invalid = handles[1];
        invalid.owner ^= 1;
        CHECK(!page_resolve(&allocator, invalid));
        CHECK(release(malloc_mode, &allocator, invalid, marker) == PAGE_INVALID);
        invalid = handles[1];
        invalid.base += PAGE_BYTES;
        CHECK(!page_resolve(&allocator, invalid));
        CHECK(release(malloc_mode, &allocator, invalid, marker) == PAGE_INVALID);
        CHECK(release(malloc_mode, &allocator, handles[1], marker) == 0);
        CHECK(!page_resolve(&allocator, handles[1]));
        CHECK(release(malloc_mode, &allocator, handles[1], marker) == PAGE_INVALID);
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, 2, marker + 3, &replacement) == 0);
        CHECK(replacement.slot == handles[1].slot);
        CHECK(!page_resolve(&allocator, handles[1]));
        CHECK(release(malloc_mode, &allocator, handles[1], marker) == PAGE_INVALID);
        CHECK(bytes_match(&allocator, replacement, marker + 3, 0));
        CHECK(bytes_match(&allocator, handles[2], marker + 2, 0));
        CHECK(release(malloc_mode, &allocator, replacement, marker) == 0);
        CHECK(release(malloc_mode, &allocator, handles[2], marker) == 0);
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, 0, marker, &replacement) == PAGE_INVALID);
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, ~0UL, marker, &replacement) == PAGE_INVALID);
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, PAGE_SINGLE_MAX + 1, marker, &replacement) == PAGE_INVALID);
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, PAGE_SINGLE_MAX, marker + 1, &handles[1]) == 0);
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, PAGE_TOTAL_MAX - PAGE_SINGLE_MAX - 1,
                      marker + 2, &handles[2]) == 0);
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, 1, marker, &replacement) == PAGE_NO_MEMORY);
        CHECK(bytes_match(&allocator, handles[1], marker + 1, 0));
        CHECK(bytes_match(&allocator, handles[2], marker + 2, 0));
        CHECK(release(malloc_mode, &allocator, handles[2], marker) == 0);
        CHECK(release(malloc_mode, &allocator, handles[1], marker) == 0);
        for (unsigned int i = 1; i < PAGE_SLOTS; ++i)
            CHECK(attempt(malloc_mode, &allocator, &guest_pages, 1, marker + i, &handles[i]) == 0);
        CHECK(attempt(malloc_mode, &allocator, &guest_pages, 1, marker, &replacement) == PAGE_NO_MEMORY);
        for (unsigned int i = 1; i < PAGE_SLOTS; ++i) {
            CHECK(bytes_match(&allocator, handles[i], marker + i, 0));
            CHECK(release(malloc_mode, &allocator, handles[i], marker) == 0);
        }
    }
    CHECK(bytes_match(&allocator, handles[0], marker, 0));
    CHECK(release(malloc_mode, &allocator, handles[0], marker) == 0);
    CHECK(!allocator.used && !allocator.pages);
#ifdef MALLOC_FIXTURE
#undef allocator
    if (malloc_mode)
        return !LOG("[MALL-ALLOC-DONE] owners=0 pages=0\n");
#endif
    return !LOG("[PAGE-ALLOC-DONE] owners=0 pages=0\n");
}
