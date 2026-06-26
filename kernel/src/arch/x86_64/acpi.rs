use alloc::vec::Vec;
use core::mem;
use x86_64::PhysAddr;

/// ACPI 2.0+ compliant RSDP (Root System Description Pointer)
/// Can be located by Limine bootloader or fallback scan in 0xE0000-0xFFFFF
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct ACPIRSDP {
    pub signature: [u8; 8], // "RSD PTR "
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub revision: u8,
    pub rsdt_addr: u32, // 32-bit pointer to RSDT (ACPI 1.0)

    // Extended fields (ACPI 2.0+)
    pub length: u32,
    pub xsdt_addr: u64, // 64-bit pointer to XSDT (ACPI 2.0+)
    pub extended_checksum: u8,
    pub reserved: [u8; 3],
}

/// Standard ACPI table header
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct ACPITableHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: u32,
    pub creator_revision: u32,
}

/// RSDT - Root System Description Table (ACPI 1.0, 32-bit)
#[repr(C, packed)]
pub struct ACPIRSDT {
    pub header: ACPITableHeader,
    pub tables: [u32; 1], // Minimum 1; actual count determined by header.length
}

/// XSDT - Extended System Description Table (ACPI 2.0+, 64-bit)
#[repr(C, packed)]
pub struct ACPIXSDT {
    pub header: ACPITableHeader,
    pub tables: [u64; 1], // Minimum 1; actual count determined by header.length
}

/// FADT - Fixed ACPI Description Table (for power control)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct ACPIFADT {
    pub header: ACPITableHeader,
    pub firmware_ctrl: u32,
    pub dsdt_addr: u32,
    pub reserved1: u8,
    pub preferred_pm_profile: u8,
    pub sci_int: u16,
    pub smi_cmd_port: u32,
    pub acpi_enable_value: u8,
    pub acpi_disable_value: u8,
    pub s4bios_req: u8,
    pub pstate_cnt: u8,
    pub pm1a_evt_blk: u32,
    pub pm1b_evt_blk: u32,
    pub pm1a_cnt_blk: u32,
    pub pm1b_cnt_blk: u32,
    pub pm2_cnt_blk: u32,
    pub pm_tmr_blk: u32,
    pub gpe0_blk: u32,
    pub gpe1_blk: u32,
    pub pm1_evt_len: u8,
    pub pm1_cnt_len: u8,
    pub pm2_cnt_len: u8,
    pub pm_tmr_len: u8,
    pub gpe0_blk_len: u8,
    pub gpe1_blk_len: u8,
    pub gpe1_base: u8,
    pub cst_cnt: u8,
    pub p_lvl2_lat: u16,
    pub p_lvl3_lat: u16,
    pub flush_size: u16,
    pub flush_stride: u16,
    pub duty_offset: u8,
    pub duty_width: u8,
    pub day_alarm: u8,
    pub month_alarm: u8,
    pub century: u8,
    pub iapc_boot_arch: u16,
    pub reserved2: u8,
    pub flags: u32,
    pub reset_reg: [u8; 12], // GenericAddressStructure
    pub reset_value: u8,
    pub reserved3: [u8; 3],
    pub firmware_ctrl_ext: u64, // ACPI 4.0+
    pub dsdt_addr_ext: u64,     // ACPI 4.0+
}

/// MADT - Multiple APIC Description Table (for multiprocessor support)
#[repr(C, packed)]
pub struct ACPIMADT {
    pub header: ACPITableHeader,
    pub local_apic_addr: u32,
    pub flags: u32,
    // Followed by variable-length APIC structures
}

/// Global ACPI state
#[derive(Clone, Copy)]
pub struct ACPIState {
    pub version: u8,
    pub rsdp_phys: u64,
    pub xsdt_phys: u64,
    pub rsdt_phys: u64,
    pub fadt_phys: u64,
    pub pm1a_cnt_blk: u32,
    pub pm1b_cnt_blk: u32,
    pub pm1_cnt_len: u8,
    pub smi_cmd_port: u32,
    pub acpi_enable_value: u8,
    pub acpi_disable_value: u8,
    pub reset_reg_addr: u64,
    pub reset_reg_is_port: bool,
    pub reset_value: u8,
    pub slp_typea: u8,
    pub slp_typeb: u8,
    /// Physical address of the MADT table (0 = not yet parsed).
    pub madt_phys: u64,
    /// Physical base address of the Local APIC MMIO region from MADT.
    pub lapic_base: u64,
}

impl ACPIState {
    pub const fn new() -> Self {
        ACPIState {
            version: 0,
            rsdp_phys: 0,
            xsdt_phys: 0,
            rsdt_phys: 0,
            fadt_phys: 0,
            pm1a_cnt_blk: 0,
            pm1b_cnt_blk: 0,
            pm1_cnt_len: 0,
            smi_cmd_port: 0,
            acpi_enable_value: 0,
            acpi_disable_value: 0,
            reset_reg_addr: 0,
            reset_reg_is_port: false,
            reset_value: 0,
            slp_typea: 0,
            slp_typeb: 0,
            madt_phys: 0,
            lapic_base: 0,
        }
    }
}

static ACPI_STATE: spin::Mutex<ACPIState> = spin::Mutex::new(ACPIState::new());

/// Compute checksum of ACPI table (sum of all bytes should be 0 mod 256)
fn verify_checksum(data: &[u8]) -> bool {
    let sum: u32 = data.iter().map(|&b| b as u32).sum();
    (sum & 0xFF) == 0
}

/// Convert 4-char signature to string for debugging
fn sig_to_str(sig: &[u8; 4]) -> &str {
    core::str::from_utf8(sig).unwrap_or("????")
}

/// Milestone 1: Physical Table Discovery & Mapping
/// Initialize ACPI tables from Limine-provided RSDP or fallback scan
pub unsafe fn init(rsdp_phys: u64) -> Result<(), &'static str> {
    if rsdp_phys == 0 {
        crate::serial_println!("[ACPI] Error: No RSDP from bootloader");
        return Err("No RSDP from bootloader");
    }

    crate::serial_println!(
        "[ACPI] RSDP structure located at physical address: {:#x}",
        rsdp_phys
    );

    // Map RSDP (should be identity-mapped in HHDM)
    let rsdp = unsafe { (rsdp_phys as *const ACPIRSDP).as_ref() }.ok_or("Failed to map RSDP")?;

    // Verify RSDP signature
    if &rsdp.signature != b"RSD PTR " {
        crate::serial_println!("[ACPI] Invalid RSDP signature");
        return Err("Invalid RSDP signature");
    }

    // Verify RSDP checksum
    let rsdp_size = if rsdp.revision == 0 { 20 } else { 36 };
    let rsdp_slice =
        unsafe { core::slice::from_raw_parts(rsdp as *const _ as *const u8, rsdp_size) };
    if !verify_checksum(rsdp_slice) {
        crate::serial_println!("[ACPI] RSDP checksum invalid");
        return Err("RSDP checksum invalid");
    }

    crate::serial_println!(
        "[ACPI] Checksum verified completely. ACPI Revision: {}.0 ({} active)",
        rsdp.revision,
        if rsdp.revision >= 2 { "XSDT" } else { "RSDT" }
    );

    let mut state = ACPI_STATE.lock();
    state.version = rsdp.revision;
    state.rsdp_phys = rsdp_phys;

    // Discover tables from XSDT (preferred) or RSDT
    if rsdp.revision >= 2 && rsdp.xsdt_addr != 0 {
        discover_tables_from_xsdt(rsdp.xsdt_addr, &mut state)?;
    } else {
        discover_tables_from_rsdt(rsdp.rsdt_addr as u64, &mut state)?;
    }

    drop(state); // Release the lock before calling parse_fadt

    // Milestone 2: Parse FADT for shutdown primitives
    parse_fadt()?;

    crate::serial_println!(
        "[ACPI] Initialization complete. System is ACPI revision {}",
        { ACPI_STATE.lock().version }
    );
    Ok(())
}

fn discover_tables_from_xsdt(xsdt_phys: u64, state: &mut ACPIState) -> Result<(), &'static str> {
    let xsdt = unsafe { (xsdt_phys as *const ACPIXSDT).as_ref() }.ok_or("Failed to map XSDT")?;

    // Verify XSDT checksum
    let xsdt_slice = unsafe {
        core::slice::from_raw_parts(xsdt as *const _ as *const u8, xsdt.header.length as usize)
    };
    if !verify_checksum(xsdt_slice) {
        crate::serial_println!("[ACPI] XSDT checksum invalid");
        return Err("XSDT checksum invalid");
    }

    state.xsdt_phys = xsdt_phys;

    let table_count = (xsdt.header.length as usize - mem::size_of::<ACPITableHeader>()) / 8;
    crate::serial_println!(
        "[ACPI] Found table: {} (Extended System Description Table) at {:#x}",
        sig_to_str(&xsdt.header.signature),
        xsdt_phys
    );

    // Walk through table pointers using pointer arithmetic (flexible array)
    for i in 0..table_count {
        // Calculate table pointer from struct base (header size + i * 8 bytes)
        let offset = mem::size_of::<ACPITableHeader>() + i * 8;
        let table_phys = unsafe {
            let table_ptr = (xsdt_phys as *const u8).add(offset) as *const u64;
            table_ptr.read_unaligned()
        };
        dump_table_header(table_phys)?;
    }

    Ok(())
}

fn discover_tables_from_rsdt(rsdt_phys: u64, state: &mut ACPIState) -> Result<(), &'static str> {
    let rsdt = unsafe { (rsdt_phys as *const ACPIRSDT).as_ref() }.ok_or("Failed to map RSDT")?;

    // Verify RSDT checksum
    let rsdt_slice = unsafe {
        core::slice::from_raw_parts(rsdt as *const _ as *const u8, rsdt.header.length as usize)
    };
    if !verify_checksum(rsdt_slice) {
        crate::serial_println!("[ACPI] RSDT checksum invalid");
        return Err("RSDT checksum invalid");
    }

    state.rsdt_phys = rsdt_phys;

    let table_count = (rsdt.header.length as usize - mem::size_of::<ACPITableHeader>()) / 4;
    crate::serial_println!(
        "[ACPI] Found table: {} (Root System Description Table) at {:#x}",
        sig_to_str(&rsdt.header.signature),
        rsdt_phys
    );

    // Walk through table pointers using pointer arithmetic (flexible array)
    for i in 0..table_count {
        // Calculate table pointer from struct base (header size + i * 4 bytes)
        let offset = mem::size_of::<ACPITableHeader>() + i * 4;
        let table_phys = unsafe {
            let table_ptr = (rsdt_phys as *const u8).add(offset) as *const u32;
            table_ptr.read_unaligned() as u64
        };
        dump_table_header(table_phys)?;
    }

    Ok(())
}

fn dump_table_header(table_phys: u64) -> Result<(), &'static str> {
    let header = unsafe { (table_phys as *const ACPITableHeader).as_ref() }
        .ok_or("Failed to map table header")?;

    // Verify table checksum
    let table_slice = unsafe {
        core::slice::from_raw_parts(header as *const _ as *const u8, header.length as usize)
    };
    if !verify_checksum(table_slice) {
        crate::serial_println!(
            "[ACPI] Warning: {} checksum invalid at {:#x}",
            sig_to_str(&header.signature),
            table_phys
        );
        // Don't return error; some tables may be corrupt
    }

    crate::serial_println!(
        "[ACPI] Found table: {} at {:#x}",
        sig_to_str(&header.signature),
        table_phys
    );

    Ok(())
}

/// Milestone 2: Parse FADT for shutdown primitives
fn parse_fadt() -> Result<(), &'static str> {
    let fadt_phys = find_table_by_signature(b"FACP")?;

    let fadt_hdr = unsafe {
        (fadt_phys as *const ACPIFADT)
            .as_ref()
            .ok_or("Failed to map FADT")?
    };

    let mut state = ACPI_STATE.lock();
    state.fadt_phys = fadt_phys;
    state.pm1a_cnt_blk = fadt_hdr.pm1a_cnt_blk;
    state.pm1b_cnt_blk = fadt_hdr.pm1b_cnt_blk;
    state.pm1_cnt_len = fadt_hdr.pm1_cnt_len;
    state.smi_cmd_port = fadt_hdr.smi_cmd_port;
    state.acpi_enable_value = fadt_hdr.acpi_enable_value;
    state.acpi_disable_value = fadt_hdr.acpi_disable_value;

    // Extract reset register info from FADT (bytes 116-127)
    let reset_reg = &fadt_hdr.reset_reg;
    // GenericAddressStructure: [0]=address_space_id, [1]=bit_width, [2]=bit_offset, [3]=reserved, [4-11]=address
    let addr_space_id = reset_reg[0];
    state.reset_reg_is_port = addr_space_id == 1; // 1 = SystemIO, 0 = SystemMemory
    state.reset_value = fadt_hdr.reset_value;

    // Extract address (little-endian 64-bit from bytes 4-11)
    let addr_bytes = &reset_reg[4..12];
    let mut reset_addr: u64 = 0;
    for (i, &byte) in addr_bytes.iter().enumerate() {
        reset_addr |= (byte as u64) << (i * 8);
    }
    state.reset_reg_addr = reset_addr;

    crate::serial_println!(
        "[ACPI] PM1a_CNT_BLK port assigned: {:#x}",
        state.pm1a_cnt_blk
    );
    crate::serial_println!(
        "[ACPI] PM1b_CNT_BLK port assigned: {:#x}",
        state.pm1b_cnt_blk
    );

    // Enable ACPI mode if needed
    if state.smi_cmd_port != 0 && state.acpi_enable_value != 0 {
        crate::serial_println!(
            "[ACPI] Enabling ACPI mode via SMI_CMD port {:#x}... Done.",
            state.smi_cmd_port
        );
        // Would write ACPI_ENABLE_VALUE to SMI_CMD_PORT here in real implementation
        // For QEMU/KVM, ACPI is often already enabled; this is a safety check
    }

    let dsdt_addr = fadt_hdr.dsdt_addr as u64;
    drop(state); // Release the lock before calling parse_dsdt

    // Milestone 3: Parse DSDT for sleep state values
    parse_dsdt(dsdt_addr)?;

    Ok(())
}

/// Decode a small AML integer operand. Returns (value, bytes_consumed).
/// Handles ZeroOp/OneOp/OnesOp/BytePrefix; falls back to treating the byte as a
/// literal for non-conforming firmware.
fn read_aml_int(bytes: &[u8]) -> (u8, usize) {
    match bytes.first().copied() {
        None => (0, 0),
        Some(0x00) => (0, 1),                                  // ZeroOp
        Some(0x01) => (1, 1),                                  // OneOp
        Some(0xFF) => (0xFF, 1),                               // OnesOp
        Some(0x0A) => (bytes.get(1).copied().unwrap_or(0), 2), // BytePrefix + value
        Some(v) => (v, 1),                                     // defensive: bare literal
    }
}

/// Milestone 3: Extract SLP_TYPx values from DSDT bytecode
fn parse_dsdt(dsdt_phys: u64) -> Result<(), &'static str> {
    if dsdt_phys == 0 {
        crate::serial_println!("[ACPI] DSDT not available (may not support S5 shutdown)");
        return Ok(()); // Not fatal
    }

    let dsdt_hdr = unsafe {
        (dsdt_phys as *const ACPITableHeader)
            .as_ref()
            .ok_or("Failed to map DSDT")?
    };

    let dsdt_bytes =
        unsafe { core::slice::from_raw_parts(dsdt_phys as *const u8, dsdt_hdr.length as usize) };

    // The _S5 object is the AML NameString "_S5_" (note the trailing underscore)
    // followed by a Package:
    //     12 <PkgLength> <NumElements> <elem0> <elem1> ...
    // where elem0 = SLP_TYPa and elem1 = SLP_TYPb. PkgLength is variable-width
    // (its top two bits give the count of extra length bytes) and each element
    // is an AML integer operand, so the operands must be decoded, not read at a
    // fixed offset. The previous code read dsdt_bytes[i+7]/[i+8], which landed on
    // NumElements/garbage and produced a bogus SLP_TYP — the reason shutdown
    // wrote the wrong value and never powered off.
    let n = dsdt_bytes.len();
    let mut i = 0usize;
    while i + 4 <= n {
        if &dsdt_bytes[i..i + 4] == b"_S5_" {
            let mut j = i + 4;
            if j < n && dsdt_bytes[j] == 0x12 {
                j += 1; // skip PackageOp
                if j < n {
                    let extra = (dsdt_bytes[j] >> 6) as usize; // PkgLength: extra byte count
                    j += 1 + extra; // skip the whole PkgLength field
                }
                j += 1; // skip NumElements
                if j < n {
                    let (slp_typea, adv) = read_aml_int(&dsdt_bytes[j..]);
                    let (slp_typeb, _) = read_aml_int(&dsdt_bytes[j + adv..]);
                    let mut state = ACPI_STATE.lock();
                    state.slp_typea = slp_typea;
                    state.slp_typeb = slp_typeb;
                    crate::serial_println!(
                        "[ACPI] _S5 Sleep Types: a={:#x}, b={:#x}",
                        slp_typea,
                        slp_typeb
                    );
                    return Ok(());
                }
            }
        }
        i += 1;
    }

    crate::serial_println!("[ACPI] _S5 object not found in DSDT (using defaults)");
    // Default S5 values for common x86 systems
    let mut state = ACPI_STATE.lock();
    state.slp_typea = 0;
    state.slp_typeb = 0;

    Ok(())
}

/// Find an ACPI table by its 4-char signature
fn find_table_by_signature(signature: &[u8; 4]) -> Result<u64, &'static str> {
    let state = ACPI_STATE.lock();
    if state.xsdt_phys != 0 {
        find_table_in_xsdt(signature, state.xsdt_phys)
    } else {
        find_table_in_rsdt(signature, state.rsdt_phys)
    }
}

fn find_table_in_xsdt(signature: &[u8; 4], xsdt_phys: u64) -> Result<u64, &'static str> {
    let xsdt = unsafe {
        (xsdt_phys as *const ACPIXSDT)
            .as_ref()
            .ok_or("Failed to map XSDT")?
    };

    let table_count = (xsdt.header.length as usize - mem::size_of::<ACPITableHeader>()) / 8;
    for i in 0..table_count {
        let offset = mem::size_of::<ACPITableHeader>() + i * 8;
        let table_phys = unsafe {
            let table_ptr = (xsdt_phys as *const u8).add(offset) as *const u64;
            table_ptr.read_unaligned()
        };
        let header = unsafe {
            (table_phys as *const ACPITableHeader)
                .as_ref()
                .ok_or("Failed to map table")?
        };
        if &header.signature == signature {
            return Ok(table_phys);
        }
    }
    Err("Table not found")
}

fn find_table_in_rsdt(signature: &[u8; 4], rsdt_phys: u64) -> Result<u64, &'static str> {
    let rsdt = unsafe {
        (rsdt_phys as *const ACPIRSDT)
            .as_ref()
            .ok_or("Failed to map RSDT")?
    };

    let table_count = (rsdt.header.length as usize - mem::size_of::<ACPITableHeader>()) / 4;
    for i in 0..table_count {
        let offset = mem::size_of::<ACPITableHeader>() + i * 4;
        let table_phys = unsafe {
            let table_ptr = (rsdt_phys as *const u8).add(offset) as *const u32;
            table_ptr.read_unaligned() as u64
        };
        let header = unsafe {
            (table_phys as *const ACPITableHeader)
                .as_ref()
                .ok_or("Failed to map table")?
        };
        if &header.signature == signature {
            return Ok(table_phys);
        }
    }
    Err("Table not found")
}

/// Milestone 4: Reboot using reset register
pub fn reboot() -> ! {
    crate::serial_println!("[ACPI] Attempting system reboot via reset register...");

    let state = ACPI_STATE.lock();
    if state.reset_reg_addr != 0 && state.reset_value != 0 {
        if state.reset_reg_is_port {
            // I/O port write
            let port = state.reset_reg_addr as u16;
            let value = state.reset_value;
            unsafe {
                core::arch::asm!(
                    "out dx, al",
                    in("dx") port,
                    in("al") value,
                );
            }
        } else {
            // Memory-mapped I/O write
            let addr = state.reset_reg_addr as *mut u8;
            unsafe {
                addr.write_volatile(state.reset_value);
            }
        }
    }

    drop(state);

    // Fallback: 8042 keyboard controller reset (common on x86)
    crate::serial_println!("[ACPI] Reset register failed, trying 8042 keyboard controller...");
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x64u16,
            in("al") 0xFEu8,
        );
    }

    // If all else fails, loop forever
    loop {
        unsafe { core::arch::asm!("hlt") }
    }
}

#[inline]
unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    core::arch::asm!(
        "in ax, dx",
        out("ax") value,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    value
}

#[inline]
unsafe fn outw(port: u16, value: u16) {
    core::arch::asm!(
        "out dx, ax",
        in("dx") port,
        in("ax") value,
        options(nomem, nostack, preserves_flags)
    );
}

#[inline]
unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags)
    );
}

/// SLP_EN bit in PM1_CNT (bit 13) and SCI_EN bit (bit 0).
const SLP_EN: u16 = 1 << 13;
const SCI_EN: u16 = 1 << 0;

/// Milestone 5: Shutdown using S5 sleep state
pub fn shutdown() -> ! {
    // Snapshot what we need, then drop the lock — we never return, and we don't
    // want to hold it across the (possibly long) ACPI-enable poll.
    let (pm1a_port, pm1b_port, slp_typa, slp_typb, smi_cmd, acpi_enable) = {
        let state = ACPI_STATE.lock();
        (
            state.pm1a_cnt_blk as u16,
            state.pm1b_cnt_blk as u16,
            state.slp_typea as u16,
            state.slp_typeb as u16,
            state.smi_cmd_port as u16,
            state.acpi_enable_value,
        )
    };

    // No timer/keyboard IRQ should run between enabling ACPI and the S5 write.
    unsafe { core::arch::asm!("cli", options(nomem, nostack)) };

    crate::serial_println!(
        "[ACPI] Shutdown: PM1a_CNT={:#x} SLP_TYPa={:#x}",
        pm1a_port,
        slp_typa
    );

    if pm1a_port == 0 {
        crate::serial_println!("[ACPI] No PM1a_CNT block discovered; cannot S5 shutdown");
    } else {
        // Real firmware hands the machine off in legacy mode (SCI_EN=0). Writing
        // SLP_EN to PM1_CNT only powers off once ACPI mode is enabled, so kick
        // SMI_CMD with ACPI_ENABLE and wait for SCI_EN to latch. QEMU usually
        // boots with SCI_EN already set, so this is a no-op there.
        if unsafe { inw(pm1a_port) } & SCI_EN == 0 && smi_cmd != 0 && acpi_enable != 0 {
            crate::serial_println!("[ACPI] Enabling ACPI mode via SMI_CMD {:#x}", smi_cmd);
            unsafe { outb(smi_cmd, acpi_enable) };
            let mut spins = 0u32;
            while unsafe { inw(pm1a_port) } & SCI_EN == 0 && spins < 3_000_000 {
                spins += 1;
                core::hint::spin_loop();
            }
        }

        // PM1_CNT is a 16-bit register: SLP_TYP occupies bits [12:10], SLP_EN is
        // bit 13. Must be a 16-bit OUT — the old 32-bit `out dx, eax` spilled
        // into the adjacent port.
        let sleep_a = (slp_typa << 10) | SLP_EN;
        crate::serial_println!(
            "[ACPI] Writing S5 {:#x} to PM1a_CNT {:#x}",
            sleep_a,
            pm1a_port
        );
        unsafe { outw(pm1a_port, sleep_a) };

        if pm1b_port != 0 {
            let sleep_b = (slp_typb << 10) | SLP_EN;
            unsafe { outw(pm1b_port, sleep_b) };
        }
    }

    crate::serial_println!("[ACPI] Shutdown initiated. Powering down...");

    // Loop forever waiting for power off
    loop {
        unsafe { core::arch::asm!("hlt") }
    }
}

/// Query current ACPI state
pub fn get_state() -> ACPIState {
    *ACPI_STATE.lock()
}

// ─────────────────────────────────────────────────────────────────────────────
// MADT (Multiple APIC Description Table) parser
//
// Must be called AFTER the kernel heap is initialized (acpi::init runs before
// the heap; parse_madt is a separate call made from main after heap setup).
// ─────────────────────────────────────────────────────────────────────────────

/// MADT entry type codes (ACPI spec §5.2.12).
const MADT_TYPE_LOCAL_APIC: u8 = 0;
const MADT_TYPE_IO_APIC: u8 = 1;
const MADT_TYPE_INT_SRC_OVERRIDE: u8 = 2;
const MADT_TYPE_LOCAL_APIC_NMI: u8 = 4;
const MADT_TYPE_X2APIC: u8 = 9;

/// Processor Local APIC entry (type 0, length = 8).
/// One entry per logical processor detected by firmware.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MadtLocalApic {
    pub entry_type: u8,
    pub length: u8,
    pub acpi_proc_uid: u8,
    pub apic_id: u8,
    /// Bit 0: Enabled. Bit 1: Online Capable (ACPI 6.3+).
    pub flags: u32,
}

/// I/O APIC entry (type 1, length = 12).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MadtIoApic {
    pub entry_type: u8,
    pub length: u8,
    pub io_apic_id: u8,
    pub _reserved: u8,
    pub io_apic_addr: u32,
    pub global_sys_int_base: u32,
}

/// Interrupt Source Override entry (type 2, length = 10).
/// Describes how ISA IRQs map to Global System Interrupts.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MadtIntSourceOverride {
    pub entry_type: u8,
    pub length: u8,
    pub bus: u8,
    pub source: u8,
    pub global_sys_int: u32,
    pub flags: u16,
}

/// Local APIC NMI connection entry (type 4, length = 6).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MadtLocalApicNmi {
    pub entry_type: u8,
    pub length: u8,
    pub acpi_proc_uid: u8,
    pub flags: u16,
    pub lint: u8,
}

/// Processor Local x2APIC entry (type 9, length = 16).
/// Used on systems with more than 255 logical processors.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MadtX2Apic {
    pub entry_type: u8,
    pub length: u8,
    pub _reserved: u16,
    pub x2apic_id: u32,
    pub flags: u32,
    pub acpi_proc_uid: u32,
}

/// A parsed MADT entry yielded by `MadtParser`.
pub enum MadtEntry {
    LocalApic(MadtLocalApic),
    IoApic(MadtIoApic),
    IntSourceOverride(MadtIntSourceOverride),
    LocalApicNmi(MadtLocalApicNmi),
    X2Apic(MadtX2Apic),
    /// Any entry type this parser does not model; carried as (type, length).
    Unknown(u8, u8),
}

/// Per-logical-processor info extracted from MADT Local APIC / x2APIC entries.
#[derive(Clone, Copy, Debug)]
pub struct CoreInfo {
    /// APIC ID used to address this processor in IPI / LAPIC operations.
    /// For xAPIC entries this fits in u8; for x2APIC entries it is a full u32.
    pub apic_id: u32,
    /// ACPI processor UID (matches `_UID` in the DSDT namespace).
    pub acpi_proc_uid: u32,
    /// Raw MADT flags word. Bit 0 = Enabled. Bit 1 = Online Capable.
    pub flags: u32,
    /// True when this entry came from a type-9 x2APIC record.
    pub is_x2apic: bool,
}

impl CoreInfo {
    /// Returns `true` if firmware reports this processor as currently enabled.
    #[inline]
    pub fn is_enabled(self) -> bool {
        self.flags & 1 != 0
    }

    /// Returns `true` if firmware can bring this processor online at runtime
    /// (ACPI 6.3+ "Online Capable" flag, bit 1).
    #[inline]
    pub fn is_online_capable(self) -> bool {
        self.flags & 2 != 0
    }

    /// A processor is usable if it is either already enabled or can be brought
    /// online. This is the correct predicate for counting available cores.
    #[inline]
    pub fn is_usable(self) -> bool {
        self.is_enabled() || self.is_online_capable()
    }
}

/// Bounds-checked iterator over MADT variable-length entries.
///
/// Holds a raw pointer to the first byte after the MADT fixed header and
/// walks forward by each entry's own `length` field. Iteration stops when
/// fewer than 2 bytes remain or a malformed length is detected.
///
/// SAFETY invariant: `base` and `table_end` must point into a single
/// contiguous, readable mapping that lives at least as long as this iterator.
pub struct MadtParser {
    base: *const u8,
    table_end: *const u8,
    offset: usize,
}

impl MadtParser {
    /// Construct a parser for the MADT whose first byte is at `madt_phys`.
    ///
    /// Returns `None` if the table is smaller than the fixed MADT header or if
    /// the physical address is null.
    ///
    /// SAFETY: `madt_phys` must be a valid physical address (identity-mapped
    /// in the HHDM) pointing to a complete, readable MADT.
    pub unsafe fn new(madt_phys: u64) -> Option<Self> {
        if madt_phys == 0 {
            return None;
        }
        let hdr = unsafe { (madt_phys as *const ACPIMADT).as_ref()? };
        let total_len = hdr.header.length as usize;
        let fixed = mem::size_of::<ACPIMADT>(); // header (36) + local_apic_addr (4) + flags (4) = 44
        if total_len < fixed {
            return None;
        }
        Some(MadtParser {
            base: unsafe { (madt_phys as *const u8).add(fixed) },
            table_end: unsafe { (madt_phys as *const u8).add(total_len) },
            offset: 0,
        })
    }

    #[inline]
    fn remaining_bytes(&self) -> usize {
        let cur = unsafe { self.base.add(self.offset) } as usize;
        (self.table_end as usize).saturating_sub(cur)
    }
}

impl Iterator for MadtParser {
    type Item = MadtEntry;

    fn next(&mut self) -> Option<MadtEntry> {
        // Every entry has at least an (entry_type: u8, length: u8) header.
        if self.remaining_bytes() < 2 {
            return None;
        }

        let ptr = unsafe { self.base.add(self.offset) };
        let entry_type = unsafe { ptr.read() };
        let length = unsafe { ptr.add(1).read() };

        // Malformed: length field is too small or overruns the table.
        if length < 2 || (length as usize) > self.remaining_bytes() {
            return None;
        }

        let entry = match (entry_type, length) {
            (MADT_TYPE_LOCAL_APIC, 8) => {
                MadtEntry::LocalApic(unsafe { (ptr as *const MadtLocalApic).read_unaligned() })
            }
            (MADT_TYPE_IO_APIC, 12) => {
                MadtEntry::IoApic(unsafe { (ptr as *const MadtIoApic).read_unaligned() })
            }
            (MADT_TYPE_INT_SRC_OVERRIDE, 10) => MadtEntry::IntSourceOverride(unsafe {
                (ptr as *const MadtIntSourceOverride).read_unaligned()
            }),
            (MADT_TYPE_LOCAL_APIC_NMI, 6) => {
                MadtEntry::LocalApicNmi(unsafe {
                    (ptr as *const MadtLocalApicNmi).read_unaligned()
                })
            }
            (MADT_TYPE_X2APIC, 16) => {
                MadtEntry::X2Apic(unsafe { (ptr as *const MadtX2Apic).read_unaligned() })
            }
            _ => MadtEntry::Unknown(entry_type, length),
        };

        self.offset += length as usize;
        Some(entry)
    }
}

/// Parse the MADT to enumerate all logical processors and the I/O APIC.
///
/// Returns a `Vec<CoreInfo>` containing one entry per Local APIC / x2APIC
/// record found in the table, regardless of enabled/disabled status. Callers
/// should filter on `CoreInfo::is_usable()` to count schedulable cores.
///
/// Also stores the MADT physical address and Local APIC MMIO base in
/// `ACPI_STATE` for use by the LAPIC driver.
///
/// SAFETY: must be called after the kernel heap is initialized (this function
/// allocates). The ACPI RSDP / XSDT / RSDT must already have been mapped by
/// `acpi::init()`.
pub unsafe fn parse_madt() -> Result<Vec<CoreInfo>, &'static str> {
    let madt_phys = find_table_by_signature(b"APIC")?;

    // Validate checksum over the entire table.
    let hdr = unsafe { (madt_phys as *const ACPIMADT).as_ref() }.ok_or("Failed to map MADT")?;
    let table_bytes = unsafe {
        core::slice::from_raw_parts(madt_phys as *const u8, hdr.header.length as usize)
    };
    if !verify_checksum(table_bytes) {
        return Err("MADT checksum invalid");
    }

    // Snapshot the Local APIC MMIO base from the fixed header.
    let lapic_base = hdr.local_apic_addr as u64;

    crate::serial_println!(
        "[ACPI] MADT: Local APIC base = {:#x}, flags = {:#x}",
        lapic_base,
        { hdr.flags }
    );

    // Commit to ACPI state so lapic_base_addr() is usable even if we return early.
    {
        let mut state = ACPI_STATE.lock();
        state.madt_phys = madt_phys;
        state.lapic_base = lapic_base;
    }

    let parser = unsafe { MadtParser::new(madt_phys) }.ok_or("MADT too small to parse")?;
    let mut cores: Vec<CoreInfo> = Vec::new();

    for entry in parser {
        match entry {
            MadtEntry::LocalApic(apic) => {
                let flags = apic.flags;
                crate::serial_println!(
                    "[ACPI] MADT:  Local APIC  proc_uid={:3}  apic_id={:3}  flags={:#x}{}",
                    apic.acpi_proc_uid,
                    apic.apic_id,
                    flags,
                    if flags & 1 != 0 { "  [enabled]" } else { "  [disabled]" }
                );
                cores.push(CoreInfo {
                    apic_id: apic.apic_id as u32,
                    acpi_proc_uid: apic.acpi_proc_uid as u32,
                    flags,
                    is_x2apic: false,
                });
            }

            MadtEntry::IoApic(io) => {
                crate::serial_println!(
                    "[ACPI] MADT:  I/O APIC    id={:3}  addr={:#x}  gsi_base={}",
                    io.io_apic_id,
                    { io.io_apic_addr },
                    { io.global_sys_int_base }
                );
            }

            MadtEntry::IntSourceOverride(iso) => {
                crate::serial_println!(
                    "[ACPI] MADT:  IRQ override bus={}  src={:3}  gsi={:3}  flags={:#x}",
                    iso.bus,
                    iso.source,
                    { iso.global_sys_int },
                    { iso.flags }
                );
            }

            MadtEntry::LocalApicNmi(nmi) => {
                crate::serial_println!(
                    "[ACPI] MADT:  LAPIC NMI   proc_uid={}  lint={}  flags={:#x}",
                    nmi.acpi_proc_uid,
                    nmi.lint,
                    { nmi.flags }
                );
            }

            MadtEntry::X2Apic(x2) => {
                let flags = x2.flags;
                // x2APIC IDs < 255 are also represented as type-0 entries on
                // well-formed firmware; skip duplicates (same proc_uid).
                let proc_uid = x2.acpi_proc_uid;
                let already_listed = cores.iter().any(|c| c.acpi_proc_uid == proc_uid);
                crate::serial_println!(
                    "[ACPI] MADT:  x2APIC      proc_uid={:3}  x2apic_id={:5}  flags={:#x}{}{}",
                    proc_uid,
                    { x2.x2apic_id },
                    flags,
                    if flags & 1 != 0 { "  [enabled]" } else { "  [disabled]" },
                    if already_listed { "  (dup, skipped)" } else { "" }
                );
                if !already_listed {
                    cores.push(CoreInfo {
                        apic_id: x2.x2apic_id,
                        acpi_proc_uid: proc_uid,
                        flags,
                        is_x2apic: true,
                    });
                }
            }

            MadtEntry::Unknown(t, l) => {
                crate::serial_println!(
                    "[ACPI] MADT:  Unknown entry  type={}  length={}",
                    t,
                    l
                );
            }
        }
    }

    let usable = cores.iter().filter(|c| c.is_usable()).count();
    crate::serial_println!(
        "[ACPI] MADT: {} logical processor(s) found, {} usable",
        cores.len(),
        usable
    );

    Ok(cores)
}

/// Returns the Local APIC MMIO base address recorded during `parse_madt()`.
/// Returns 0 if `parse_madt()` has not been called yet.
pub fn lapic_base_addr() -> u64 {
    ACPI_STATE.lock().lapic_base
}
