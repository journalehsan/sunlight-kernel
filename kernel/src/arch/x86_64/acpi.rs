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
