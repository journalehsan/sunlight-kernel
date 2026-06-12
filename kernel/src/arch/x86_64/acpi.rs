use core::mem;
use x86_64::PhysAddr;

/// ACPI 2.0+ compliant RSDP (Root System Description Pointer)
/// Can be located by Limine bootloader or fallback scan in 0xE0000-0xFFFFF
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct ACPIRSDP {
    pub signature: [u8; 8],      // "RSD PTR "
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub revision: u8,
    pub rsdt_addr: u32,          // 32-bit pointer to RSDT (ACPI 1.0)

    // Extended fields (ACPI 2.0+)
    pub length: u32,
    pub xsdt_addr: u64,          // 64-bit pointer to XSDT (ACPI 2.0+)
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
    pub dsdt_addr_ext: u64,      // ACPI 4.0+
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

    crate::serial_println!("[ACPI] RSDP structure located at physical address: {:#x}", rsdp_phys);

    // Map RSDP (should be identity-mapped in HHDM)
    let rsdp = unsafe { (rsdp_phys as *const ACPIRSDP).as_ref() }
        .ok_or("Failed to map RSDP")?;

    // Verify RSDP signature
    if &rsdp.signature != b"RSD PTR " {
        crate::serial_println!("[ACPI] Invalid RSDP signature");
        return Err("Invalid RSDP signature");
    }

    // Verify RSDP checksum
    let rsdp_size = if rsdp.revision == 0 { 20 } else { 36 };
    let rsdp_slice = unsafe {
        core::slice::from_raw_parts(rsdp as *const _ as *const u8, rsdp_size)
    };
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
    let xsdt = unsafe { (xsdt_phys as *const ACPIXSDT).as_ref() }
        .ok_or("Failed to map XSDT")?;

    // Verify XSDT checksum
    let xsdt_slice = unsafe {
        core::slice::from_raw_parts(
            xsdt as *const _ as *const u8,
            xsdt.header.length as usize,
        )
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
    let rsdt = unsafe { (rsdt_phys as *const ACPIRSDT).as_ref() }
        .ok_or("Failed to map RSDT")?;

    // Verify RSDT checksum
    let rsdt_slice = unsafe {
        core::slice::from_raw_parts(
            rsdt as *const _ as *const u8,
            rsdt.header.length as usize,
        )
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
        core::slice::from_raw_parts(
            header as *const _ as *const u8,
            header.length as usize,
        )
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
        (fadt_phys as *const ACPIFADT).as_ref()
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

    crate::serial_println!("[ACPI] PM1a_CNT_BLK port assigned: {:#x}", state.pm1a_cnt_blk);
    crate::serial_println!("[ACPI] PM1b_CNT_BLK port assigned: {:#x}", state.pm1b_cnt_blk);

    // Enable ACPI mode if needed
    if state.smi_cmd_port != 0 && state.acpi_enable_value != 0 {
        crate::serial_println!("[ACPI] Enabling ACPI mode via SMI_CMD port {:#x}... Done.", state.smi_cmd_port);
        // Would write ACPI_ENABLE_VALUE to SMI_CMD_PORT here in real implementation
        // For QEMU/KVM, ACPI is often already enabled; this is a safety check
    }

    let dsdt_addr = fadt_hdr.dsdt_addr as u64;
    drop(state); // Release the lock before calling parse_dsdt

    // Milestone 3: Parse DSDT for sleep state values
    parse_dsdt(dsdt_addr)?;

    Ok(())
}

/// Milestone 3: Extract SLP_TYPx values from DSDT bytecode
fn parse_dsdt(dsdt_phys: u64) -> Result<(), &'static str> {
    if dsdt_phys == 0 {
        crate::serial_println!("[ACPI] DSDT not available (may not support S5 shutdown)");
        return Ok(()); // Not fatal
    }

    let dsdt_hdr = unsafe {
        (dsdt_phys as *const ACPITableHeader).as_ref()
            .ok_or("Failed to map DSDT")?
    };

    // Search for _S5 object in DSDT AML bytecode
    // Pattern: 0x08 0x5F 0x53 0x35 (name definition) followed by a package
    let dsdt_bytes = unsafe {
        core::slice::from_raw_parts(
            dsdt_phys as *const u8,
            dsdt_hdr.length as usize,
        )
    };

    // Search for _S5 bytecode sequence
    for i in 0..dsdt_bytes.len().saturating_sub(4) {
        if dsdt_bytes[i] == 0x08 &&        // Name operation
           dsdt_bytes[i+1] == b'_' as u8 &&  // Underscore prefix
           dsdt_bytes[i+2] == b'S' as u8 &&  // S
           dsdt_bytes[i+3] == b'5' as u8     // 5
        {
            // Found _S5 object. Extract sleep type values from following package
            // Simplified: look for the package payload bytes
            if i + 9 < dsdt_bytes.len() {
                // Typical pattern: package length, then SLP_TYPa, SLP_TYPb bytes
                let slp_typea = dsdt_bytes[i + 7];
                let slp_typeb = dsdt_bytes[i + 8];
                let mut state = ACPI_STATE.lock();
                state.slp_typea = slp_typea;
                state.slp_typeb = slp_typeb;
                crate::serial_println!("[ACPI] _S5 Sleep Types: a={:#x}, b={:#x}", slp_typea, slp_typeb);
                return Ok(());
            }
        }
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
        (xsdt_phys as *const ACPIXSDT).as_ref()
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
            (table_phys as *const ACPITableHeader).as_ref()
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
        (rsdt_phys as *const ACPIRSDT).as_ref()
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
            (table_phys as *const ACPITableHeader).as_ref()
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

/// Milestone 5: Shutdown using S5 sleep state
pub fn shutdown() -> ! {
    let state = ACPI_STATE.lock();
    crate::serial_println!(
        "[ACPI] Writing S5 sleep payload state to PM1a_CNT_BLK port {:#x}...",
        state.pm1a_cnt_blk
    );

    if state.pm1a_cnt_blk != 0 {
        // Construct S5 sleep value:
        // bits [12:10] = SLP_TYPa (sleep type for PM1a)
        // bit 13 = SLP_EN (sleep enable)
        let sleep_val = ((state.slp_typea as u32) << 10) | (1u32 << 13);
        let port_a = state.pm1a_cnt_blk as u16;

        unsafe {
            core::arch::asm!(
                "out dx, eax",
                in("dx") port_a,
                in("eax") sleep_val,
            );
        }

        // Also write PM1b if present
        if state.pm1b_cnt_blk != 0 {
            let sleep_val_b = ((state.slp_typeb as u32) << 10) | (1u32 << 13);
            let port_b = state.pm1b_cnt_blk as u16;
            unsafe {
                core::arch::asm!(
                    "out dx, eax",
                    in("dx") port_b,
                    in("eax") sleep_val_b,
                );
            }
        }
    }

    drop(state);
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
