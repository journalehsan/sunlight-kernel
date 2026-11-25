#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "bounded-malloc.h"

static unsigned long mapped[PAGE_SLOTS], calls;
static unsigned char backing[PAGE_SLOTS][HELIOS_MALLOC_MAX];
static long map_error, unmap_error;

static long map(unsigned long bytes)
{
    ++calls;
    if (map_error)
        return map_error;
    for (unsigned long slot = 0; slot < PAGE_SLOTS; ++slot) {
        if (!mapped[slot]) {
            mapped[slot] = bytes;
            memset(backing[slot], 0, bytes);
            return (long)((slot + 1) * HELIOS_MALLOC_MAX);
        }
    }
    __builtin_trap();
}

static long unmap(unsigned long base, unsigned long bytes)
{
    unsigned long slot = base / HELIOS_MALLOC_MAX - 1;
    assert(slot < PAGE_SLOTS && mapped[slot] == bytes);
    if (unmap_error)
        return unmap_error;
    mapped[slot] = 0;
    return 0;
}

static int reject(unsigned long base, unsigned long bytes)
{
    assert(base % HELIOS_MALLOC_ALIGNMENT == 0 && bytes == 2 * PAGE_BYTES);
    return 0;
}

static void __attribute__((noreturn)) fatal(void)
{
    __builtin_trap();
}

static const struct page_backend backend = {map, unmap, 0, fatal};

static struct helios_heap *staged_heap;

static int reject_staged(unsigned long base, unsigned long bytes)
{
    struct page_handle staged = staged_heap->pages.slots[1];
    assert(staged.base == base && staged.pages * PAGE_BYTES == bytes);
    assert(page_resolve(&staged_heap->pages, staged) == base);
    const struct helios_allocation unpublished = {staged, PAGE_BYTES + 1};
    assert(!helios_resolve(staged_heap, unpublished));
    assert(helios_free(staged_heap, &backend, unpublished) == PAGE_INVALID);
    assert(staged_heap->pages.used == 2 && staged_heap->pages.pages == 3);
    return reject(base, bytes);
}

static struct helios_heap fresh(void)
{
    for (unsigned long i = 0; i < PAGE_SLOTS; ++i)
        assert(!mapped[i]);
    calls = 0;
    map_error = unmap_error = 0;
    return (struct helios_heap){.pages.owner = 41};
}

static struct helios_allocation allocate(struct helios_heap *heap, unsigned long size)
{
    struct helios_allocation allocation;
    assert(!helios_malloc(heap, &backend, size, 0, &allocation));
    assert(allocation.requested == size);
    assert(helios_resolve(heap, allocation) == allocation.backing.base);
    assert(allocation.backing.base % HELIOS_MALLOC_ALIGNMENT == 0);
    assert(heap->slots[allocation.backing.slot].alignment == HELIOS_MALLOC_ALIGNMENT);
    return allocation;
}

static void release(struct helios_heap *heap, struct helios_allocation allocation)
{
    assert(!helios_free(heap, &backend, allocation));
    assert(!heap->slots[allocation.backing.slot].requested);
    assert(!heap->slots[allocation.backing.slot].alignment);
}

static void sizes(void)
{
    struct helios_heap heap = fresh();
    struct helios_allocation out = {.requested = 42};
    assert(helios_malloc(&heap, &backend, 0, 0, &out) == PAGE_INVALID);
    assert(helios_malloc(&heap, &backend, ~0UL, 0, &out) == PAGE_INVALID);
    assert(helios_malloc(&heap, &backend, HELIOS_MALLOC_MAX + 1, 0, &out) == PAGE_INVALID);
    assert(!calls && out.requested == 42);
    struct helios_allocation one = allocate(&heap, 1);
    struct helios_allocation two = allocate(&heap, PAGE_BYTES + 1);
    assert(one.backing.pages == 1 && two.backing.pages == 2);
    assert(one.backing.base + PAGE_BYTES <= two.backing.base);
    backing[one.backing.slot][0] = 0x61;
    backing[two.backing.slot][PAGE_BYTES] = 0x62;
    release(&heap, one);
    struct helios_allocation reuse = allocate(&heap, 19);
    assert(reuse.backing.slot == one.backing.slot && reuse.backing.base == one.backing.base);
    assert(reuse.backing.generation > one.backing.generation);
    assert(!backing[reuse.backing.slot][0]);
    assert(backing[two.backing.slot][PAGE_BYTES] == 0x62);
    assert(helios_free(&heap, &backend, one) == PAGE_INVALID);
    release(&heap, two);
    release(&heap, reuse);
}

static void identities(void)
{
    struct helios_heap heap = fresh(), foreign = {.pages.owner = 42};
    struct helios_allocation live = allocate(&heap, PAGE_BYTES + 19);
    assert(helios_free(&foreign, &backend, live) == PAGE_INVALID);
    assert(helios_free(&heap, &backend, (struct helios_allocation){0}) == PAGE_INVALID);
    struct helios_allocation invalid = live;
    invalid.backing.base += PAGE_BYTES;
    assert(helios_free(&heap, &backend, invalid) == PAGE_INVALID);
    invalid.backing.base = live.backing.base + 1;
    assert(helios_free(&heap, &backend, invalid) == PAGE_INVALID);
    invalid = live;
    ++invalid.requested;
    assert(helios_free(&heap, &backend, invalid) == PAGE_INVALID);
    invalid = live;
    ++invalid.backing.generation;
    assert(helios_free(&heap, &backend, invalid) == PAGE_INVALID);
    assert(heap.pages.used == 1 && heap.pages.pages == 2);
    release(&heap, live);
    assert(helios_free(&heap, &backend, live) == PAGE_INVALID);
}

static void capacity(void)
{
    struct helios_heap heap = fresh();
    struct helios_allocation slots[PAGE_SLOTS], out = {0};
    for (unsigned long i = 0; i < PAGE_SLOTS; ++i) {
        slots[i] = allocate(&heap, i + 1);
        assert(slots[i].backing.slot == i);
    }
    assert(helios_malloc(&heap, &backend, 1, 0, &out) == PAGE_NO_MEMORY);
    assert(calls == PAGE_SLOTS && heap.pages.pages == PAGE_SLOTS);
    for (unsigned long i = 0; i < PAGE_SLOTS; ++i)
        release(&heap, slots[i]);
    slots[0] = allocate(&heap, HELIOS_MALLOC_MAX);
    slots[1] = allocate(&heap, HELIOS_MALLOC_MAX);
    assert(helios_malloc(&heap, &backend, 1, 0, &out) == PAGE_NO_MEMORY);
    assert(calls == PAGE_SLOTS + 2 && heap.pages.pages == PAGE_TOTAL_MAX);
    release(&heap, slots[0]);
    release(&heap, slots[1]);
}

static void rollback(long error, int backing_gate, int shim_gate)
{
    struct helios_heap heap = fresh();
    struct helios_allocation live = allocate(&heap, 17), out = {.requested = 42};
    backing[live.backing.slot][0] = 0x71;
    struct page_backend fault = backend;
    if (backing_gate)
        fault.publish = reject;
    else if (!shim_gate)
        map_error = error;
    staged_heap = &heap;
    assert(helios_malloc(&heap, &fault, PAGE_BYTES + 1, shim_gate ? reject_staged : 0, &out) == error);
    staged_heap = 0;
    assert(out.requested == 42 && !out.backing.base);
    assert(heap.pages.used == 1 && heap.pages.pages == 1 && heap.pages.generation == 1);
    assert(!heap.slots[1].requested && !heap.slots[1].alignment && !mapped[1] && !heap.pages.slots[1].generation);
    assert(mapped[0] == PAGE_BYTES && helios_resolve(&heap, live));
    assert(backing[live.backing.slot][0] == 0x71);
    map_error = 0;
    release(&heap, live);
}

static void free_failure(void)
{
    struct helios_heap heap = fresh();
    struct helios_allocation live = allocate(&heap, 19);
    unmap_error = PAGE_NO_MEMORY;
    assert(helios_free(&heap, &backend, live) == PAGE_NO_MEMORY);
    assert(heap.pages.used == 1 && heap.pages.pages == 1 && heap.slots[0].requested == 19);
    assert(mapped[0] == PAGE_BYTES && helios_resolve(&heap, live));
    unmap_error = 0;
    release(&heap, live);
}

int main(void)
{
    sizes();
    puts("PASS size rounding, alignment, non-overlap and non-LIFO reuse");
    identities();
    puts("PASS zero, stale, duplicate, foreign, interior and mismatched handles");
    capacity();
    puts("PASS table capacity and total backing budget");
    rollback(PAGE_NO_MEMORY, 0, 0);
    puts("PASS backing memory failure");
    rollback(PAGE_INVALID, 0, 0);
    puts("PASS backing mapping publication error");
    rollback(PAGE_PUBLICATION, 1, 0);
    puts("PASS backing metadata publication rollback");
    rollback(PAGE_PUBLICATION, 0, 1);
    puts("PASS shim publication rollback after backing acquisition");
    free_failure();
    puts("PASS recoverable backing unmap failure");
    puts("8 passed; 0 failed");
}
