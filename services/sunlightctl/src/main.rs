//! sunlightctl - Control interface for sunlightd
//! Thin IPC client for managing services

#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 16384] = [0; 16384];
        static mut NEXT: usize = 0;
        let start = NEXT;
        let align = layout.align();
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

use sunlight_ipc::{IpcMsg, ipc_call, nameserver_lookup, CapabilityToken};

const SERIAL_PORT: u16 = 0x3f8;

fn serial_out(s: &str) {
    for byte in s.as_bytes() {
        unsafe {
            core::arch::asm!(
                "out dx, al",
                in("dx") SERIAL_PORT,
                in("al") *byte,
                options(nomem, nostack)
            );
        }
    }
}

macro_rules! println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        serial_out(&buf);
        serial_out("\n");
    }};
}

fn print_usage() {
    println!("Usage: sunlightctl <command> [unit]");
    println!("Commands:");
    println!("  start <unit>    - Start a service");
    println!("  stop <unit>     - Stop a service");
    println!("  restart <unit>  - Restart a service");
    println!("  status <unit>   - Show service status");
    println!("  list            - List all services");
    println!("  reload          - Reload service definitions");
}

fn pack_unit_name(msg: &mut IpcMsg, name: &str) {
    let bytes = name.as_bytes();
    for i in 0..4 {
        let mut word: u64 = 0;
        for j in 0..8 {
            let idx = i * 8 + j;
            if idx < bytes.len() {
                word |= (bytes[idx] as u64) << (j * 8);
            }
        }
        msg.words[i] = word;
    }
}

fn extract_unit_name(msg: &IpcMsg) -> heapless::String<64> {
    let mut name = heapless::String::new();
    
    for i in 0..4 {
        let word = msg.words[i];
        for j in 0..8 {
            let byte = ((word >> (j * 8)) & 0xff) as u8;
            if byte == 0 {
                return name;
            }
            let _ = name.push(byte as char);
        }
    }
    
    name
}

fn cmd_status(sunlightd_cap: CapabilityToken, unit: &str) {
    let mut msg = IpcMsg::empty();
    msg.label = 10; // Status
    pack_unit_name(&mut msg, unit);

    let reply = ipc_call(sunlightd_cap, msg);
    
    if reply.label == 1 {
        let state = reply.words[0] as u32;
        let pid = reply.words[1] as u32;
        let restarts = reply.words[2] as u32;
        let started_at = reply.words[3];

        println!("● {}.service", unit);
        println!("   Active: {}", match state {
            0 => "stopped",
            1 => "starting",
            2 => "running",
            3 => "failed",
            4 => "restarting",
            _ => "unknown",
        });
        if state == 2 {
            println!("   PID: {}", pid);
            println!("   Started: {}", started_at);
        }
        println!("   Restarts: {}", restarts);
    } else {
        println!("ERROR: Service not found or status unavailable");
    }
}

fn cmd_list(sunlightd_cap: CapabilityToken) {
    let mut msg = IpcMsg::empty();
    msg.label = 11; // List

    let reply = ipc_call(sunlightd_cap, msg);
    
    if reply.label == 1 {
        println!("UNIT               STATE     PID   RESTARTS");
        
        let name = extract_unit_name(&reply);
        let state = reply.words[4] as u32;
        let pid = reply.words[5] as u32;
        let restarts = reply.words[6] as u32;

        let state_str = match state {
            2 => "running",
            0 => "stopped",
            _ => "unknown",
        };

        println!("{:<18} {:<9} {:<5} {}", name, state_str, pid, restarts);
    } else {
        println!("ERROR: List unavailable");
    }
}

fn cmd_start(sunlightd_cap: CapabilityToken, unit: &str) {
    let mut msg = IpcMsg::empty();
    msg.label = 1; // Start
    pack_unit_name(&mut msg, unit);

    let reply = ipc_call(sunlightd_cap, msg);
    
    if reply.label == 1 {
        println!("Started {}.service", unit);
    } else {
        println!("ERROR: Failed to start service");
    }
}

fn cmd_stop(sunlightd_cap: CapabilityToken, unit: &str) {
    let mut msg = IpcMsg::empty();
    msg.label = 2; // Stop
    pack_unit_name(&mut msg, unit);

    let reply = ipc_call(sunlightd_cap, msg);
    
    if reply.label == 1 {
        println!("Stopped {}.service", unit);
    } else {
        println!("ERROR: Failed to stop service");
    }
}

fn cmd_restart(sunlightd_cap: CapabilityToken, unit: &str) {
    let mut msg = IpcMsg::empty();
    msg.label = 3; // Restart
    pack_unit_name(&mut msg, unit);

    let reply = ipc_call(sunlightd_cap, msg);
    
    if reply.label == 1 {
        println!("Restarted {}.service", unit);
    } else {
        println!("ERROR: Failed to restart service");
    }
}

fn cmd_reload(sunlightd_cap: CapabilityToken) {
    let mut msg = IpcMsg::empty();
    msg.label = 4; // Reload

    let reply = ipc_call(sunlightd_cap, msg);
    
    if reply.label == 1 {
        println!("Reloaded service definitions");
    } else {
        println!("ERROR: Failed to reload");
    }
}

#[no_mangle]
fn _start() -> ! {
    // TODO: Parse command line arguments from argc/argv
    // For now, hardcoded to run 'list' command for testing
    
    // Lookup sunlightd capability
    let sunlightd_cap = nameserver_lookup("sunlightd");
    if sunlightd_cap.is_none() {
        println!("ERROR: sunlightd not found (is it running?)");
        loop {}
    }

    // Run list command
    cmd_list(sunlightd_cap.unwrap());

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    println!("sunlightctl: PANIC: {}", _info);
    loop {}
}
