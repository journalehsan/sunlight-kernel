//! SMBIOS discovery via Limine and bounded parse into public identity.
//!
//! Discovery preference (UEFI and BIOS both go through Limine):
//! - Limine `SmbiosRequest` supplies 32-bit and/or 64-bit entry-point pointers.
//! - Prefer validated SMBIOS 3.x when both are valid.
//! - Map the structure table through HHDM only after entry-point validation.
//! - Do not log serial numbers or UUIDs.

use core::sync::atomic::{AtomicBool, Ordering};
use spin::Mutex;
use sunlight_smbios::{
    parse_public_identity, select_entry_point, validate_entry_point_32, validate_entry_point_64,
    EntryPointKind, IdentityConfidence, PrivilegedUniqueIds, ProcessorInfo, PublicSystemIdentity,
    SmbiosError, MAX_TABLE_BYTES,
};
use x86_64::VirtAddr;

struct SmbiosState {
    public: PublicSystemIdentity,
    privileged: PrivilegedUniqueIds,
    processor: ProcessorInfo,
    entry_kind: Option<EntryPointKind>,
    ready: bool,
}

impl SmbiosState {
    const fn empty() -> Self {
        Self {
            public: PublicSystemIdentity::empty(),
            privileged: PrivilegedUniqueIds::empty(),
            processor: ProcessorInfo::empty(),
            entry_kind: None,
            ready: false,
        }
    }
}

static STATE: Mutex<SmbiosState> = Mutex::new(SmbiosState::empty());
static INIT_DONE: AtomicBool = AtomicBool::new(false);

/// Physical pointers from Limine (may be 0 if absent).
pub struct LimineSmbiosPointers {
    pub entry_32_phys: u64,
    pub entry_64_phys: u64,
}

/// Initialize SMBIOS identity from Limine-provided entry points.
///
/// `hhdm_offset` is used to form a virtual mapping of firmware physical memory.
/// Invalid entry points are discarded; no partial untrusted identity is retained.
pub unsafe fn init(hhdm: VirtAddr, pointers: LimineSmbiosPointers) {
    if INIT_DONE.swap(true, Ordering::SeqCst) {
        return;
    }

    let ep32 = if pointers.entry_32_phys != 0 {
        match read_and_validate_32(hhdm, pointers.entry_32_phys) {
            Ok(v) => Some(v),
            Err(e) => {
                crate::serial_println!("[SMBIOS] 32-bit entry rejected: {:?}", e);
                None
            }
        }
    } else {
        None
    };

    let ep64 = if pointers.entry_64_phys != 0 {
        match read_and_validate_64(hhdm, pointers.entry_64_phys) {
            Ok(v) => Some(v),
            Err(e) => {
                crate::serial_println!("[SMBIOS] 64-bit entry rejected: {:?}", e);
                None
            }
        }
    } else {
        None
    };

    let Some(ep) = select_entry_point(ep32, ep64) else {
        crate::serial_println!("[SMBIOS] no valid entry point");
        return;
    };

    let kind_str = match ep.kind {
        EntryPointKind::Legacy32 => "2.x",
        EntryPointKind::Smbios3 => "3.x",
    };
    crate::serial_println!(
        "[SMBIOS] using {} entry point v{}.{} table_phys={:#x} len={}",
        kind_str,
        ep.major,
        ep.minor,
        ep.table_address,
        ep.table_length
    );

    if ep.table_length as usize > MAX_TABLE_BYTES || ep.table_length == 0 {
        crate::serial_println!("[SMBIOS] table size rejected");
        return;
    }

    let table = match map_table(hhdm, ep.table_address, ep.table_length as usize) {
        Ok(t) => t,
        Err(e) => {
            crate::serial_println!("[SMBIOS] table map failed: {:?}", e);
            return;
        }
    };

    match parse_public_identity(table, &ep) {
        Ok((public, privileged, processor)) => {
            // Public boot log only — never serial/UUID.
            if public.has_product_identity() {
                crate::serial_println!(
                    "[SMBIOS] system: {} {}",
                    public.manufacturer.as_str(),
                    public.product_name.as_str()
                );
            } else {
                crate::serial_println!("[SMBIOS] system identity incomplete/unknown");
            }
            if !public.bios_vendor.is_empty() {
                crate::serial_println!(
                    "[SMBIOS] bios: {} {}",
                    public.bios_vendor.as_str(),
                    public.bios_version.as_str()
                );
            }
            let conf = match public.identity_confidence {
                IdentityConfidence::None => "none",
                IdentityConfidence::Partial => "partial",
                IdentityConfidence::Full => "full",
            };
            crate::serial_println!("[SMBIOS] identity_confidence={}", conf);

            let mut st = STATE.lock();
            st.public = public;
            st.privileged = privileged;
            st.processor = processor;
            st.entry_kind = Some(ep.kind);
            st.ready = true;
            // privileged retained in-kernel only for future privileged sysinfo;
            // not exposed by default syscalls.
            let _ = &st.privileged;
        }
        Err(e) => {
            crate::serial_println!("[SMBIOS] parse failed: {:?}", e);
        }
    }
}

unsafe fn read_and_validate_32(
    hhdm: VirtAddr,
    phys: u64,
) -> Result<sunlight_smbios::ValidatedEntryPoint, SmbiosError> {
    let virt = phys_to_virt(hhdm, phys)?;
    // Entry point is at most 32 bytes for modern layouts; read 0x20.
    let bytes = core::slice::from_raw_parts(virt as *const u8, 0x20);
    validate_entry_point_32(bytes)
}

unsafe fn read_and_validate_64(
    hhdm: VirtAddr,
    phys: u64,
) -> Result<sunlight_smbios::ValidatedEntryPoint, SmbiosError> {
    let virt = phys_to_virt(hhdm, phys)?;
    let bytes = core::slice::from_raw_parts(virt as *const u8, 0x20);
    validate_entry_point_64(bytes)
}

unsafe fn map_table(hhdm: VirtAddr, phys: u64, len: usize) -> Result<&'static [u8], SmbiosError> {
    if len == 0 || len > MAX_TABLE_BYTES {
        return Err(SmbiosError::OversizedTable);
    }
    let end = phys.checked_add(len as u64).ok_or(SmbiosError::Overflow)?;
    if end < phys {
        return Err(SmbiosError::Overflow);
    }
    let virt = phys_to_virt(hhdm, phys)?;
    Ok(core::slice::from_raw_parts(virt as *const u8, len))
}

fn phys_to_virt(hhdm: VirtAddr, phys: u64) -> Result<u64, SmbiosError> {
    // Limine base revision 3/4: addresses are physical. Older revisions may
    // already be virtual (HHDM). Detect and normalize.
    let h = hhdm.as_u64();
    let p = if phys >= h { phys - h } else { phys };
    p.checked_add(h).ok_or(SmbiosError::Overflow)
}

/// Snapshot public system identity (no serial/UUID).
pub fn public_identity() -> PublicSystemIdentity {
    STATE.lock().public
}

pub fn is_ready() -> bool {
    STATE.lock().ready
}

pub fn entry_kind() -> Option<EntryPointKind> {
    STATE.lock().entry_kind
}

/// Processor descriptive metadata (not trusted over CPUID topology).
pub fn processor_info() -> ProcessorInfo {
    STATE.lock().processor
}

/// Exact product allowlist match helper for future ThinkPad backends.
pub fn matches_product(manufacturer: &str, product: &str) -> bool {
    sunlight_smbios::matches_product_allowlist(&public_identity(), manufacturer, product)
}
