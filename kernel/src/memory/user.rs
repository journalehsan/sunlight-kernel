use crate::process::Process;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::structures::paging::{Page, PageTableFlags, Size4KiB};
use x86_64::VirtAddr;

pub const USER_END_EXCLUSIVE: u64 = 0x0000_8000_0000_0000;
pub const USER_MAX_ADDRESS: u64 = USER_END_EXCLUSIVE - 1;
const PAGE_SIZE: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserMemoryError {
    InvalidAddress,
    NonCanonical,
    Overflow,
    KernelRange,
    Unmapped,
    NotUserAccessible,
    NotWritable,
    SwappedUnsupported,
    StringTooLong,
    ArrayTooLarge,
    AllocationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserAddress(u64);

impl UserAddress {
    pub fn new(address: u64) -> Result<Self, UserMemoryError> {
        if address >= USER_END_EXCLUSIVE {
            if is_canonical(address) {
                KERNEL_RANGE_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
                return Err(UserMemoryError::KernelRange);
            }
            NONCANONICAL_ADDRESSES.fetch_add(1, Ordering::Relaxed);
            return Err(UserMemoryError::NonCanonical);
        }
        Ok(Self(address))
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserRange {
    start: u64,
    len: usize,
}

impl UserRange {
    pub fn new(start: u64, len: usize) -> Result<Self, UserMemoryError> {
        UserAddress::new(start)?;
        if len == 0 {
            return Ok(Self { start, len });
        }
        if start == 0 {
            return Err(UserMemoryError::InvalidAddress);
        }
        let end = start.checked_add((len - 1) as u64).ok_or_else(|| {
            RANGE_OVERFLOWS.fetch_add(1, Ordering::Relaxed);
            UserMemoryError::Overflow
        })?;
        UserAddress::new(end)?;
        Ok(Self { start, len })
    }

    pub fn for_elements(
        start: u64,
        count: usize,
        element_size: usize,
    ) -> Result<Self, UserMemoryError> {
        let len = count.checked_mul(element_size).ok_or_else(|| {
            RANGE_OVERFLOWS.fetch_add(1, Ordering::Relaxed);
            UserMemoryError::Overflow
        })?;
        Self::new(start, len)
    }

    pub const fn start(self) -> u64 {
        self.start
    }

    pub const fn len(self) -> usize {
        self.len
    }

    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct UserSlice(UserRange);

impl UserSlice {
    pub fn new(address: u64, len: usize) -> Result<Self, UserMemoryError> {
        UserRange::new(address, len).map(Self)
    }

    pub fn copy_into(
        self,
        process: &Process,
        hhdm: VirtAddr,
        destination: &mut [u8],
    ) -> Result<(), UserMemoryError> {
        if destination.len() != self.0.len() {
            return Err(UserMemoryError::InvalidAddress);
        }
        copy_from_process(process, hhdm, self.0, destination)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct UserSliceMut(UserRange);

impl UserSliceMut {
    pub fn new(address: u64, len: usize) -> Result<Self, UserMemoryError> {
        UserRange::new(address, len).map(Self)
    }

    pub fn copy_from(
        self,
        process: &Process,
        hhdm: VirtAddr,
        source: &[u8],
    ) -> Result<(), UserMemoryError> {
        if source.len() != self.0.len() {
            return Err(UserMemoryError::InvalidAddress);
        }
        copy_to_process(process, hhdm, self.0, source)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    Read,
    Write,
}

fn is_canonical(address: u64) -> bool {
    address <= 0x0000_7fff_ffff_ffff || address >= 0xffff_8000_0000_0000
}

fn validate_pages(
    process: &Process,
    hhdm: VirtAddr,
    range: UserRange,
    access: Access,
) -> Result<(), UserMemoryError> {
    if range.is_empty() {
        return Ok(());
    }
    let end = range
        .start
        .checked_add((range.len - 1) as u64)
        .ok_or(UserMemoryError::Overflow)?;
    let first = range.start & !0xfff;
    let last = end & !0xfff;
    let mut page_address = first;
    loop {
        let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(page_address))
            .map_err(|_| UserMemoryError::InvalidAddress)?;
        let Some((_, flags)) = (unsafe { process.address_space.lookup_entry(page, hhdm) }) else {
            UNMAPPED_COPY_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
            return Err(UserMemoryError::Unmapped);
        };
        if !flags.contains(PageTableFlags::PRESENT) {
            if unsafe { process.address_space.swapped_block_id(page, hhdm).is_some() } {
                return Err(UserMemoryError::SwappedUnsupported);
            }
            UNMAPPED_COPY_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
            return Err(UserMemoryError::Unmapped);
        }
        if !flags.contains(PageTableFlags::USER_ACCESSIBLE) {
            return Err(UserMemoryError::NotUserAccessible);
        }
        if access == Access::Write && !flags.contains(PageTableFlags::WRITABLE) {
            WRITE_TO_READ_ONLY.fetch_add(1, Ordering::Relaxed);
            return Err(UserMemoryError::NotWritable);
        }
        if page_address == last {
            break;
        }
        page_address = page_address
            .checked_add(PAGE_SIZE as u64)
            .ok_or(UserMemoryError::Overflow)?;
    }
    Ok(())
}

fn copy_from_process(
    process: &Process,
    hhdm: VirtAddr,
    range: UserRange,
    destination: &mut [u8],
) -> Result<(), UserMemoryError> {
    validate_pages(process, hhdm, range, Access::Read)?;
    if range.is_empty() {
        return Ok(());
    }
    let multi_page = (range.start & !0xfff) != (range.start + (range.len - 1) as u64) & !0xfff;
    let mut copied = 0usize;
    while copied < range.len {
        let address = range.start + copied as u64;
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(address));
        let (physical, _) = unsafe { process.address_space.lookup_entry(page, hhdm) }
            .ok_or(UserMemoryError::Unmapped)?;
        let page_offset = (address & 0xfff) as usize;
        let chunk = (PAGE_SIZE - page_offset).min(range.len - copied);
        let source = (hhdm.as_u64() + physical.as_u64() + page_offset as u64) as *const u8;
        unsafe {
            core::ptr::copy_nonoverlapping(source, destination.as_mut_ptr().add(copied), chunk);
        }
        copied += chunk;
    }
    if multi_page {
        MULTI_PAGE_COPIES.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

fn copy_to_process(
    process: &Process,
    hhdm: VirtAddr,
    range: UserRange,
    source: &[u8],
) -> Result<(), UserMemoryError> {
    validate_pages(process, hhdm, range, Access::Write)?;
    if range.is_empty() {
        return Ok(());
    }
    let multi_page = (range.start & !0xfff) != (range.start + (range.len - 1) as u64) & !0xfff;
    let mut copied = 0usize;
    while copied < range.len {
        let address = range.start + copied as u64;
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(address));
        let (physical, _) = unsafe { process.address_space.lookup_entry(page, hhdm) }
            .ok_or(UserMemoryError::Unmapped)?;
        let page_offset = (address & 0xfff) as usize;
        let chunk = (PAGE_SIZE - page_offset).min(range.len - copied);
        let destination = (hhdm.as_u64() + physical.as_u64() + page_offset as u64) as *mut u8;
        unsafe {
            core::ptr::copy_nonoverlapping(source.as_ptr().add(copied), destination, chunk);
        }
        copied += chunk;
    }
    if multi_page {
        MULTI_PAGE_COPIES.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

pub fn copy_from_current(address: u64, destination: &mut [u8]) -> Result<(), UserMemoryError> {
    let range = UserRange::new(address, destination.len())?;
    let hhdm = current_hhdm()?;
    let scheduler = crate::sched::SCHEDULER.lock();
    copy_from_process(scheduler.current_process(), hhdm, range, destination)
}

/// Copy from the current process into kernel-owned raw storage.
///
/// # Safety
///
/// `destination` must be valid for writes of `len` bytes, must not alias user
/// memory, and must remain valid for the duration of this call. The user range
/// is still validated page-by-page before the first byte is copied.
pub unsafe fn copy_from_current_raw(
    address: u64,
    destination: *mut u8,
    len: usize,
) -> Result<(), UserMemoryError> {
    if len == 0 {
        return UserRange::new(address, 0).map(|_| ());
    }
    if destination.is_null() {
        return Err(UserMemoryError::InvalidAddress);
    }
    let buffer = unsafe { core::slice::from_raw_parts_mut(destination, len) };
    copy_from_current(address, buffer)
}

pub fn copy_to_current(address: u64, source: &[u8]) -> Result<(), UserMemoryError> {
    let range = UserRange::new(address, source.len())?;
    let hhdm = current_hhdm()?;
    let scheduler = crate::sched::SCHEDULER.lock();
    copy_to_process(scheduler.current_process(), hhdm, range, source)
}

pub fn copy_from_process_bytes(
    process: &Process,
    hhdm: VirtAddr,
    address: u64,
    destination: &mut [u8],
) -> Result<(), UserMemoryError> {
    let range = UserRange::new(address, destination.len())?;
    copy_from_process(process, hhdm, range, destination)
}

pub fn validate_process_read(
    process: &Process,
    hhdm: VirtAddr,
    address: u64,
    len: usize,
) -> Result<(), UserMemoryError> {
    validate_pages(process, hhdm, UserRange::new(address, len)?, Access::Read)
}

pub fn validate_process_write(
    process: &Process,
    hhdm: VirtAddr,
    address: u64,
    len: usize,
) -> Result<(), UserMemoryError> {
    validate_pages(process, hhdm, UserRange::new(address, len)?, Access::Write)
}

pub fn validate_current_write(address: u64, len: usize) -> Result<(), UserMemoryError> {
    let hhdm = current_hhdm()?;
    let scheduler = crate::sched::SCHEDULER.lock();
    validate_process_write(scheduler.current_process(), hhdm, address, len)
}

pub fn copy_to_process_bytes(
    process: &Process,
    hhdm: VirtAddr,
    address: u64,
    source: &[u8],
) -> Result<(), UserMemoryError> {
    let range = UserRange::new(address, source.len())?;
    copy_to_process(process, hhdm, range, source)
}

pub fn read_value<const N: usize>(address: u64) -> Result<[u8; N], UserMemoryError> {
    let mut bytes = [0u8; N];
    copy_from_current(address, &mut bytes)?;
    Ok(bytes)
}

pub fn write_value<const N: usize>(address: u64, bytes: &[u8; N]) -> Result<(), UserMemoryError> {
    copy_to_current(address, bytes)
}

pub fn read_c_string(address: u64, max_len: usize) -> Result<Vec<u8>, UserMemoryError> {
    if max_len == 0 {
        BOUNDED_STRING_FAILURES.fetch_add(1, Ordering::Relaxed);
        return Err(UserMemoryError::StringTooLong);
    }
    UserAddress::new(address)?;
    if address == 0 {
        return Err(UserMemoryError::InvalidAddress);
    }
    let hhdm = current_hhdm()?;
    let scheduler = crate::sched::SCHEDULER.lock();
    read_c_string_from_process(scheduler.current_process(), hhdm, address, max_len)
}

pub fn read_c_string_from_process(
    process: &Process,
    hhdm: VirtAddr,
    address: u64,
    max_len: usize,
) -> Result<Vec<u8>, UserMemoryError> {
    if max_len == 0 {
        BOUNDED_STRING_FAILURES.fetch_add(1, Ordering::Relaxed);
        return Err(UserMemoryError::StringTooLong);
    }
    UserAddress::new(address)?;
    if address == 0 {
        return Err(UserMemoryError::InvalidAddress);
    }
    let mut result = Vec::new();
    result
        .try_reserve(max_len.min(4096))
        .map_err(|_| UserMemoryError::AllocationFailed)?;
    let mut consumed = 0usize;
    while consumed < max_len {
        let current = address
            .checked_add(consumed as u64)
            .ok_or(UserMemoryError::Overflow)?;
        let mut byte = [0u8; 1];
        copy_from_process(process, hhdm, UserRange::new(current, 1)?, &mut byte)?;
        if byte[0] == 0 {
            return Ok(result);
        }
        result.push(byte[0]);
        consumed += 1;
    }
    BOUNDED_STRING_FAILURES.fetch_add(1, Ordering::Relaxed);
    Err(UserMemoryError::StringTooLong)
}

pub fn read_pointer_array(address: u64, max_entries: usize) -> Result<Vec<u64>, UserMemoryError> {
    if address == 0 {
        return Err(UserMemoryError::InvalidAddress);
    }
    let scanned_entries = max_entries
        .checked_add(1)
        .ok_or(UserMemoryError::Overflow)?;
    let _ = UserRange::for_elements(address, scanned_entries, core::mem::size_of::<u64>())?;
    let mut result = Vec::new();
    result
        .try_reserve(max_entries)
        .map_err(|_| UserMemoryError::AllocationFailed)?;
    for index in 0..=max_entries {
        let offset = index
            .checked_mul(core::mem::size_of::<u64>())
            .ok_or(UserMemoryError::Overflow)?;
        let element_address = address
            .checked_add(offset as u64)
            .ok_or(UserMemoryError::Overflow)?;
        let bytes = read_value::<8>(element_address)?;
        let pointer = u64::from_ne_bytes(bytes);
        if pointer == 0 {
            return Ok(result);
        }
        if index == max_entries {
            break;
        }
        UserAddress::new(pointer)?;
        result.push(pointer);
    }
    POINTER_ARRAY_LIMIT_FAILURES.fetch_add(1, Ordering::Relaxed);
    Err(UserMemoryError::ArrayTooLarge)
}

fn current_hhdm() -> Result<VirtAddr, UserMemoryError> {
    crate::HHDM_REQ
        .response()
        .map(|response| VirtAddr::new(response.offset))
        .ok_or(UserMemoryError::Unmapped)
}

static NONCANONICAL_ADDRESSES: AtomicU64 = AtomicU64::new(0);
static RANGE_OVERFLOWS: AtomicU64 = AtomicU64::new(0);
static KERNEL_RANGE_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static UNMAPPED_COPY_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static WRITE_TO_READ_ONLY: AtomicU64 = AtomicU64::new(0);
static BOUNDED_STRING_FAILURES: AtomicU64 = AtomicU64::new(0);
static POINTER_ARRAY_LIMIT_FAILURES: AtomicU64 = AtomicU64::new(0);
static MULTI_PAGE_COPIES: AtomicU64 = AtomicU64::new(0);

pub fn diagnostics() -> [u64; 8] {
    [
        NONCANONICAL_ADDRESSES.load(Ordering::Relaxed),
        RANGE_OVERFLOWS.load(Ordering::Relaxed),
        KERNEL_RANGE_ATTEMPTS.load(Ordering::Relaxed),
        UNMAPPED_COPY_ATTEMPTS.load(Ordering::Relaxed),
        WRITE_TO_READ_ONLY.load(Ordering::Relaxed),
        BOUNDED_STRING_FAILURES.load(Ordering::Relaxed),
        POINTER_ARRAY_LIMIT_FAILURES.load(Ordering::Relaxed),
        MULTI_PAGE_COPIES.load(Ordering::Relaxed),
    ]
}

pub fn run_address_policy_self_tests() {
    assert!(UserAddress::new(0x400000).is_ok());
    assert!(UserAddress::new(USER_MAX_ADDRESS).is_ok());
    assert_eq!(
        UserAddress::new(USER_END_EXCLUSIVE),
        Err(UserMemoryError::NonCanonical)
    );
    assert_eq!(
        UserAddress::new(0x0001_0000_0000_0000),
        Err(UserMemoryError::NonCanonical)
    );
    assert_eq!(
        UserRange::new(0x1000, usize::MAX),
        Err(UserMemoryError::Overflow)
    );
    assert_eq!(
        UserRange::new(USER_MAX_ADDRESS, 2),
        Err(UserMemoryError::NonCanonical)
    );
    assert!(UserRange::new(0, 0).is_ok());
    assert_eq!(UserRange::new(0, 1), Err(UserMemoryError::InvalidAddress));
    assert_eq!(
        UserRange::for_elements(0x1000, usize::MAX, 8),
        Err(UserMemoryError::Overflow)
    );
    assert_eq!(
        UserAddress::new(0xffff_8000_0000_0000),
        Err(UserMemoryError::KernelRange)
    );
    crate::serial_println!("[MM-1] user address/range arithmetic: OK");
}

pub fn run_mapping_self_tests(pmm: &mut crate::memory::pmm::PhysicalMemoryManager, hhdm: VirtAddr) {
    use x86_64::structures::paging::{PageTableFlags, PhysFrame};

    let free_before = pmm.free_page_count();
    let mut process = unsafe { crate::process::Process::new(0xB201, 0, "mm1-user", pmm, hhdm) };
    let first_phys = pmm.alloc_frame().expect("MM-1 first frame");
    let spacer_phys = pmm.alloc_frame().expect("MM-1 spacer frame");
    let second_phys = pmm.alloc_frame().expect("MM-1 second frame");
    let readonly_phys = pmm.alloc_frame().expect("MM-1 read-only frame");
    let swapped_phys = pmm.alloc_frame().expect("MM-1 swapped frame");
    let flags =
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;
    let readonly_flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    let base = 0x0000_0002_1000_0000;
    let first_page = Page::<Size4KiB>::from_start_address(VirtAddr::new(base)).unwrap();
    let second_page =
        Page::<Size4KiB>::from_start_address(VirtAddr::new(base + PAGE_SIZE as u64)).unwrap();
    let readonly_page =
        Page::<Size4KiB>::from_start_address(VirtAddr::new(base + 2 * PAGE_SIZE as u64)).unwrap();
    let swapped_page =
        Page::<Size4KiB>::from_start_address(VirtAddr::new(base + 4 * PAGE_SIZE as u64)).unwrap();
    unsafe {
        process.address_space.map_page(
            first_page,
            PhysFrame::from_start_address_unchecked(first_phys),
            flags,
            pmm,
            hhdm,
        );
        process.address_space.map_page(
            second_page,
            PhysFrame::from_start_address_unchecked(second_phys),
            flags,
            pmm,
            hhdm,
        );
        process.address_space.map_page(
            readonly_page,
            PhysFrame::from_start_address_unchecked(readonly_phys),
            readonly_flags,
            pmm,
            hhdm,
        );
        process.address_space.map_page(
            swapped_page,
            PhysFrame::from_start_address_unchecked(swapped_phys),
            flags,
            pmm,
            hhdm,
        );
    }
    unsafe {
        core::ptr::write_bytes(
            (hhdm + first_phys.as_u64()).as_mut_ptr::<u8>(),
            0,
            PAGE_SIZE,
        );
        core::ptr::write_bytes(
            (hhdm + second_phys.as_u64()).as_mut_ptr::<u8>(),
            0,
            PAGE_SIZE,
        );
        core::ptr::write_bytes(
            (hhdm + readonly_phys.as_u64()).as_mut_ptr::<u8>(),
            0xA5,
            PAGE_SIZE,
        );
    }

    let input = *b"single-page";
    unsafe {
        core::ptr::copy_nonoverlapping(
            input.as_ptr(),
            (hhdm + first_phys.as_u64()).as_mut_ptr::<u8>(),
            input.len(),
        );
    }
    let mut single = [0u8; 11];
    copy_from_process_bytes(&process, hhdm, base, &mut single).unwrap();
    assert_eq!(&single, b"single-page");

    copy_to_process_bytes(&process, hhdm, base + 32, b"copy-out").unwrap();
    let mut copied_out = [0u8; 8];
    copy_from_process_bytes(&process, hhdm, base + 32, &mut copied_out).unwrap();
    assert_eq!(&copied_out, b"copy-out");

    unsafe {
        core::ptr::copy_nonoverlapping(
            b"cross-".as_ptr(),
            (hhdm + first_phys.as_u64() + 4090).as_mut_ptr::<u8>(),
            6,
        );
        core::ptr::copy_nonoverlapping(
            b"page\0".as_ptr(),
            (hhdm + second_phys.as_u64()).as_mut_ptr::<u8>(),
            5,
        );
    }
    let text = read_c_string_from_process(&process, hhdm, base + 4090, 32).unwrap();
    assert_eq!(&text, b"cross-page");

    let mut cross_page = [0u8; 11];
    copy_from_process_bytes(&process, hhdm, base + 4090, &mut cross_page).unwrap();
    assert_eq!(&cross_page, b"cross-page\0");

    let mut unmapped = [0u8; 16];
    assert_eq!(
        copy_from_process_bytes(
            &process,
            hhdm,
            base + 3 * PAGE_SIZE as u64 - 8,
            &mut unmapped,
        ),
        Err(UserMemoryError::Unmapped)
    );

    let before_first: [u8; 8] = unsafe {
        core::slice::from_raw_parts((hhdm + second_phys.as_u64() + 4088).as_ptr::<u8>(), 8)
    }
    .try_into()
    .unwrap();
    let before_second: [u8; 8] =
        unsafe { core::slice::from_raw_parts((hhdm + readonly_phys.as_u64()).as_ptr::<u8>(), 8) }
            .try_into()
            .unwrap();
    assert_eq!(
        copy_to_process_bytes(&process, hhdm, base + 2 * PAGE_SIZE as u64 - 8, &[0x5A; 16],),
        Err(UserMemoryError::NotWritable)
    );
    let after_first: [u8; 8] = unsafe {
        core::slice::from_raw_parts((hhdm + second_phys.as_u64() + 4088).as_ptr::<u8>(), 8)
    }
    .try_into()
    .unwrap();
    let after_second: [u8; 8] =
        unsafe { core::slice::from_raw_parts((hhdm + readonly_phys.as_u64()).as_ptr::<u8>(), 8) }
            .try_into()
            .unwrap();
    assert_eq!(before_first, after_first);
    assert_eq!(before_second, after_second);

    assert!(unsafe {
        process
            .address_space
            .mark_swapped(swapped_page, 0x1234, hhdm)
    });
    pmm.free_frame(swapped_phys);
    let mut swapped = [0u8; 1];
    assert_eq!(
        copy_from_process_bytes(&process, hhdm, base + 4 * PAGE_SIZE as u64, &mut swapped,),
        Err(UserMemoryError::SwappedUnsupported)
    );

    unsafe {
        process.address_space.reclaim_user_space(pmm, hhdm, true);
    }
    pmm.free_frame(spacer_phys);
    assert_eq!(pmm.free_page_count(), free_before);
    crate::serial_println!(
        "[MM-1] page-aware copy, strings, permissions, atomic output, and swap policy: OK"
    );
}
