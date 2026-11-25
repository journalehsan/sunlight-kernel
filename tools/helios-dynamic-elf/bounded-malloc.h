#ifndef HELIOS_BOUNDED_MALLOC_H
#define HELIOS_BOUNDED_MALLOC_H

#include "page-allocator.h"

enum {
    HELIOS_MALLOC_ALIGNMENT = PAGE_BYTES,
    HELIOS_MALLOC_MAX = PAGE_SINGLE_MAX * PAGE_BYTES
};

struct helios_allocation {
    struct page_handle backing;
    unsigned long requested;
};

struct helios_slot {
    unsigned long requested;
    unsigned long alignment;
};

struct helios_heap {
    struct page_allocator pages;
    /* A nonzero requested size publishes the corresponding backing slot. */
    struct helios_slot slots[PAGE_SLOTS];
};

static inline unsigned long helios_resolve(const struct helios_heap *heap,
                                           struct helios_allocation allocation)
{
    struct page_handle handle = allocation.backing;
    unsigned long base = page_resolve(&heap->pages, handle);
    if (!base || !allocation.requested)
        return 0;
    const struct helios_slot *slot = &heap->slots[handle.slot];
    if (allocation.requested != slot->requested || slot->alignment != HELIOS_MALLOC_ALIGNMENT ||
        base % slot->alignment)
        return 0;
    return base;
}

/* The optional fixture gate runs after page_alloc commits its backing slot,
 * before publishing the allocation. A failing rollback terminates the guest.
 */
static inline long helios_malloc(struct helios_heap *heap,
                                 const struct page_backend *backend,
                                 unsigned long size,
                                 int (*publish)(unsigned long, unsigned long),
                                 struct helios_allocation *out)
{
    unsigned long rounded;
    if (!size || __builtin_add_overflow(size, PAGE_BYTES - 1UL, &rounded) ||
        size > HELIOS_MALLOC_MAX)
        return PAGE_INVALID;

    struct page_handle backing;
    unsigned long generation = heap->pages.generation;
    long result = page_alloc(&heap->pages, backend, rounded / PAGE_BYTES, &backing);
    if (result)
        return result;
    if (publish && !publish(backing.base, backing.pages * PAGE_BYTES)) {
        if (page_free(&heap->pages, backend, backing))
            backend->fatal();
        heap->pages.generation = generation;
        return PAGE_PUBLICATION;
    }
    heap->slots[backing.slot] = (struct helios_slot){size, HELIOS_MALLOC_ALIGNMENT};
    *out = (struct helios_allocation){backing, size};
    return 0;
}

static inline long helios_free(struct helios_heap *heap,
                               const struct page_backend *backend,
                               struct helios_allocation allocation)
{
    if (!helios_resolve(heap, allocation))
        return PAGE_INVALID;
    long result = page_free(&heap->pages, backend, allocation.backing);
    if (!result)
        heap->slots[allocation.backing.slot] = (struct helios_slot){0};
    return result;
}

#endif
