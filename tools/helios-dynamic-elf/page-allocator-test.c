#include <assert.h>
#include <stdio.h>
#include "page-allocator.h"

static unsigned long mapped[PAGE_SLOTS];
static unsigned long map_calls, unmap_calls;
static long map_error, unmap_error;
static int publication_allowed;

static long map(unsigned long bytes)
{
    ++map_calls;
    if (map_error)
        return map_error;
    for (unsigned long i = 0; i < PAGE_SLOTS; ++i) {
        if (!mapped[i]) {
            mapped[i] = bytes;
            return (long)((i + 1) * PAGE_SINGLE_MAX * PAGE_BYTES);
        }
    }
    assert(0);
}

static long unmap(unsigned long base, unsigned long bytes)
{
    ++unmap_calls;
    unsigned long slot = base / (PAGE_SINGLE_MAX * PAGE_BYTES) - 1;
    assert(slot < PAGE_SLOTS && mapped[slot] == bytes);
    if (unmap_error)
        return unmap_error;
    mapped[slot] = 0;
    return 0;
}

static int publish(unsigned long base, unsigned long bytes)
{
    assert(base && bytes);
    return publication_allowed;
}

static void __attribute__((noreturn)) fatal(void)
{
    __builtin_trap();
}

static const struct page_backend backend = {map, unmap, publish, fatal};

static struct page_allocator fresh(void)
{
    for (unsigned long i = 0; i < PAGE_SLOTS; ++i)
        assert(!mapped[i]);
    map_calls = unmap_calls = 0;
    map_error = unmap_error = 0;
    publication_allowed = 1;
    return (struct page_allocator){.owner = 41};
}

static void ownership(void)
{
    struct page_allocator allocator = fresh();
    struct page_handle handle;
    assert(!page_alloc(&allocator, &backend, 3, &handle));
    assert(page_resolve(&allocator, handle) == handle.base);
    assert(allocator.used == 1 && allocator.pages == 3 && map_calls == 1);
    assert(!page_free(&allocator, &backend, handle));
    assert(!allocator.used && !allocator.pages && unmap_calls == 1);
}

static void invalid_handles(void)
{
    struct page_allocator allocator = fresh();
    struct page_allocator foreign = {.owner = 42};
    struct page_handle handle, replacement, invalid;
    assert(!page_alloc(&allocator, &backend, 2, &handle));
    assert(!page_resolve(&foreign, handle));
    assert(page_free(&foreign, &backend, handle) == PAGE_INVALID);
    invalid = handle;
    invalid.base += PAGE_BYTES;
    assert(!page_resolve(&allocator, invalid));
    assert(page_free(&allocator, &backend, invalid) == PAGE_INVALID);
    invalid = handle;
    ++invalid.pages;
    assert(page_free(&allocator, &backend, invalid) == PAGE_INVALID);
    invalid = handle;
    invalid.slot = PAGE_SLOTS;
    assert(page_free(&allocator, &backend, invalid) == PAGE_INVALID);
    assert(!unmap_calls);
    assert(!page_free(&allocator, &backend, handle));
    assert(page_free(&allocator, &backend, handle) == PAGE_INVALID);
    assert(!page_alloc(&allocator, &backend, 2, &replacement));
    assert(replacement.slot == handle.slot && replacement.base == handle.base);
    assert(replacement.generation != handle.generation);
    assert(!page_resolve(&allocator, handle));
    assert(page_free(&allocator, &backend, handle) == PAGE_INVALID);
    assert(!page_free(&allocator, &backend, replacement));
    assert(unmap_calls == 2);
}

static void limits(void)
{
    struct page_allocator allocator = fresh();
    struct page_handle handles[PAGE_SLOTS], rejected = {0};
    assert(page_alloc(&allocator, &backend, 0, &rejected) == PAGE_INVALID);
    assert(page_alloc(&allocator, &backend, ~0UL, &rejected) == PAGE_INVALID);
    assert(page_alloc(&allocator, &backend, PAGE_SINGLE_MAX + 1, &rejected) == PAGE_INVALID);
    assert(!map_calls);
    for (unsigned long i = 0; i < PAGE_SLOTS; ++i)
        assert(!page_alloc(&allocator, &backend, 1, &handles[i]));
    assert(page_alloc(&allocator, &backend, 1, &rejected) == PAGE_NO_MEMORY);
    assert(allocator.pages == PAGE_SLOTS && map_calls == PAGE_SLOTS);
    for (unsigned long i = 0; i < PAGE_SLOTS; ++i)
        assert(!page_free(&allocator, &backend, handles[i]));
    assert(!page_alloc(&allocator, &backend, PAGE_SINGLE_MAX, &handles[0]));
    assert(!page_alloc(&allocator, &backend, PAGE_TOTAL_MAX - PAGE_SINGLE_MAX, &handles[1]));
    assert(page_alloc(&allocator, &backend, 1, &rejected) == PAGE_NO_MEMORY);
    assert(allocator.pages == PAGE_TOTAL_MAX && allocator.used == 2 && map_calls == PAGE_SLOTS + 2);
    assert(!page_free(&allocator, &backend, handles[0]));
    assert(!page_free(&allocator, &backend, handles[1]));
    allocator.generation = ~0UL;
    assert(page_alloc(&allocator, &backend, 1, &rejected) == PAGE_NO_MEMORY);
    assert(!rejected.generation && map_calls == PAGE_SLOTS + 2);
}

static void rollback(long injected_error, int reject_publication)
{
    struct page_allocator allocator = fresh();
    struct page_handle live, rejected = {0};
    assert(!page_alloc(&allocator, &backend, 1, &live));
    map_error = injected_error;
    publication_allowed = !reject_publication;
    assert(page_alloc(&allocator, &backend, 3, &rejected) ==
           (reject_publication ? PAGE_PUBLICATION : injected_error));
    assert(allocator.used == 1 && allocator.pages == 1 && allocator.generation == 1);
    assert(!rejected.generation && !page_resolve(&allocator, rejected));
    assert(page_free(&allocator, &backend, rejected) == PAGE_INVALID);
    assert(page_resolve(&allocator, live) == live.base);
    assert(mapped[0] == PAGE_BYTES && !mapped[1]);
    assert(unmap_calls == (unsigned long)reject_publication);
    publication_allowed = 1;
    map_error = 0;
    assert(!page_alloc(&allocator, &backend, 3, &rejected));
    assert(rejected.generation == 2 && rejected.slot == 1);
    assert(!page_free(&allocator, &backend, rejected));
    assert(!page_free(&allocator, &backend, live));
}

static void free_failure(void)
{
    struct page_allocator allocator = fresh();
    struct page_handle live;
    assert(!page_alloc(&allocator, &backend, 2, &live));
    unmap_error = PAGE_NO_MEMORY;
    assert(page_free(&allocator, &backend, live) == PAGE_NO_MEMORY);
    assert(page_resolve(&allocator, live) == live.base);
    assert(allocator.used == 1 && allocator.pages == 2 && mapped[0] == 2 * PAGE_BYTES);
    unmap_error = 0;
    assert(!page_free(&allocator, &backend, live));
    assert(unmap_calls == 2 && !allocator.used && !allocator.pages && !mapped[0]);
}

int main(void)
{
    ownership();
    puts("PASS insertion, removal and ownership");
    invalid_handles();
    puts("PASS stale, duplicate, foreign, interior and malformed handles");
    limits();
    puts("PASS metadata, page budget, size, overflow and generation limits");
    rollback(PAGE_NO_MEMORY, 0);
    puts("PASS mmap ENOMEM rollback (including frame and table error propagation)");
    rollback(PAGE_INVALID, 0);
    puts("PASS mapping-publication error propagation");
    rollback(0, 1);
    puts("PASS metadata-publication rollback after mmap");
    free_failure();
    puts("PASS recoverable munmap failure");
    puts("7 passed; 0 failed");
}
