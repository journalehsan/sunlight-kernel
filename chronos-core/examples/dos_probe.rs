//! Host-only diagnostic runner. Guest writes stay in private DriveTable memory.
use chronos_core::{CpuProfile, DosDrive, GuestState, Runtime};
use std::{env, fs, path::Path, process};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        return Err("usage: dos_probe /path/to/guest/PROGRAM.COM-or-EXE".into());
    }
    let executable = Path::new(&args[1]);
    let image = fs::read(executable)?;
    println!(
        "HOST PROBE ONLY: {} ({} bytes)",
        executable.display(),
        image.len()
    );
    if let Ok(header) = chronos_core::parse_mz(&image) {
        println!(
            "MZ header={header:?} image_paragraphs={:04x}",
            (header.image_size + 15) / 16
        );
    }
    let mut runtime = match Runtime::from_program(&image, &[]) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("LOAD BLOCKED: {error:?}");
            process::exit(2);
        }
    };
    runtime.set_cpu_profile(CpuProfile::I80386RealMode);
    runtime.set_executable_path(
        format!("C:\\{}", executable.file_name().unwrap().to_string_lossy()).as_bytes(),
    );
    // Only sibling regular files are imported; no symlinks, recursion, or
    // guest-requested host opens. No changes are persisted to the host.
    for entry in fs::read_dir(executable.parent().unwrap())? {
        let entry = entry?;
        if entry.file_type()?.is_file() && entry.metadata()?.len() <= 2 * 1024 * 1024 {
            runtime
                .drives_mut()
                .add_base_file(
                    DosDrive::C,
                    &entry.file_name().to_string_lossy(),
                    fs::read(entry.path())?,
                )
                .map_err(|error| format!("import failed: {error:?}"))?;
        }
    }
    println!(
        "initial PSP={:04x} end={:04x}",
        runtime.cpu.ds,
        runtime.memory.read_u16(runtime.cpu.ds, 2)
    );
    println!("initial CPU={:04x?}", runtime.cpu);
    dump_arena(&runtime);
    let mut previous_heap = None;
    let mut previous_vector = None;
    let mut previous_system_vector = None;
    let mut system_vector_changes = 0;
    let trace_system = env::var_os("CHRONOS_TRACE_INT15").is_some();
    let detail_start = env::var("CHRONOS_DETAIL_START")
        .ok()
        .and_then(|s| s.parse::<u64>().ok());
    let detail_end = detail_start.map(|start| start.saturating_add(1000));
    let mut recovered_child_trap = None;
    let trace = env::var_os("CHRONOS_TRACE").is_some();
    let mut io_trace_remaining = if env::var_os("CHRONOS_TRACE_IO").is_some() {
        128
    } else {
        0
    };
    for _ in 0..if trace { 10_000_000 } else { 1000 } {
        if trace {
            if trace_system {
                let vector = (
                    runtime.memory.read_u16(0, 0x56),
                    runtime.memory.read_u16(0, 0x54),
                );
                if previous_system_vector != Some(vector) {
                    system_vector_changes += 1;
                    if system_vector_changes <= 128 {
                        println!(
                            "IVT 15 retired={} vector={:04x}:{:04x} CPU={:04x?}",
                            runtime.instructions_retired(),
                            vector.0,
                            vector.1,
                            runtime.cpu
                        );
                    }
                    previous_system_vector = Some(vector);
                }
            }
            let vector = (
                runtime.memory.read_u16(0, 0xbe),
                runtime.memory.read_u16(0, 0xbc),
            );
            if previous_vector != Some(vector) {
                println!("IVT 2f retired={} physical=000bc vector={:04x}:{:04x} PSP={:04x} state={:?} CPU={:04x?}", runtime.instructions_retired(), vector.0, vector.1, runtime.current_psp(), runtime.state(), runtime.cpu);
                previous_vector = Some(vector);
            }
            // DN-5 diagnostic-only watch: observe guest heap initialization, never modify it.
            if runtime.instructions_retired() > 3_000_000 {
                let heap: Vec<_> = (0x95ce..0x95de)
                    .step_by(2)
                    .map(|offset| runtime.memory.read_u16(runtime.cpu.ds, offset))
                    .collect();
                if previous_heap.as_ref() != Some(&(runtime.cpu.ds, heap.clone())) {
                    println!(
                        "heap watch retired={} DS={:04x} words[95ce..95dc]={heap:04x?} CPU={:04x?}",
                        runtime.instructions_retired(),
                        runtime.cpu.ds,
                        runtime.cpu
                    );
                    previous_heap = Some((runtime.cpu.ds, heap));
                }
            }
            if detail_start
                .zip(detail_end)
                .is_some_and(|(start, end)| (start..end).contains(&runtime.instructions_retired()))
            {
                let bytes: Vec<_> = (0..8)
                    .map(|i| {
                        runtime
                            .memory
                            .read_u8(runtime.cpu.cs, runtime.cpu.ip.wrapping_add(i))
                    })
                    .collect();
                if trace_system {
                    let cpu = runtime.cpu;
                    let frame = core::array::from_fn::<_, 5, _>(|word| {
                        runtime
                            .memory
                            .read_u16(cpu.ss, cpu.sp.wrapping_add(2 * word as u16))
                    });
                    println!("trace stack={frame:04x?} IVT15={previous_system_vector:04x?}");
                }
                println!(
                    "detail retired={} bytes={bytes:02x?} {:04x?}",
                    runtime.instructions_retired(),
                    runtime.cpu
                );
            }
            let opcode = runtime.memory.read_u8(runtime.cpu.cs, runtime.cpu.ip);
            let interrupt = runtime
                .memory
                .read_u8(runtime.cpu.cs, runtime.cpu.ip.wrapping_add(1));
            if opcode == 0xc0 || opcode == 0xc1 {
                println!(
                    "before shift retired={} {:04x?}",
                    runtime.instructions_retired(),
                    runtime.cpu
                );
            }
            if opcode == 0xcd && interrupt == 0x2f {
                let bytes: Vec<_> = (0..8)
                    .map(|i| {
                        runtime
                            .memory
                            .read_u8(runtime.cpu.cs, runtime.cpu.ip.wrapping_add(i))
                    })
                    .collect();
                let stack: Vec<_> = (0..12)
                    .map(|i| {
                        runtime
                            .memory
                            .read_u16(runtime.cpu.ss, runtime.cpu.sp.wrapping_add(i * 2))
                    })
                    .collect();
                println!("multiplex bytes={bytes:02x?} stack={stack:04x?} PSP={:04x} vector={previous_vector:04x?} state={:?}", runtime.current_psp(), runtime.state());
            }
            if opcode == 0xcd {
                println!(
                    "before INT {interrupt:02x} retired={} {:04x?}",
                    runtime.instructions_retired(),
                    runtime.cpu
                );
            }
            if opcode == 0xcd && interrupt == 0x16 {
                println!(
                    "INT 16 diagnostic retired={} AH={:02x} ZF={} queued_keys={} FLAGS={:04x}",
                    runtime.instructions_retired(),
                    runtime.cpu.ah(),
                    runtime.cpu.flags & chronos_core::CpuState::FLAG_ZF != 0,
                    runtime.queued_key_count(),
                    runtime.cpu.flags,
                );
            }
            if opcode == 0xcd && interrupt == 0x10 {
                dump_video(&runtime, "before");
            }
            let multiplex_iret = opcode == 0xcf
                && previous_vector.is_some_and(|(segment, _)| runtime.cpu.cs == segment);
            if multiplex_iret {
                println!(
                    "before multiplex IRET retired={} CPU={:04x?}",
                    runtime.instructions_retired(),
                    runtime.cpu
                );
            }
            let io_before = if io_trace_remaining > 0 && matches!(opcode, 0xe4..=0xe7 | 0xec..=0xef)
            {
                io_trace_remaining -= 1;
                let port = if opcode < 0xec {
                    u16::from(interrupt)
                } else {
                    runtime.cpu.dx
                };
                Some((runtime.instructions_retired(), runtime.cpu, port))
            } else {
                None
            };
            let xlat_before = if opcode == 0x26
                && runtime
                    .memory
                    .read_u8(runtime.cpu.cs, runtime.cpu.ip.wrapping_add(1))
                    == 0xd7
            {
                let cpu = runtime.cpu;
                let offset = cpu.bx.wrapping_add(u16::from(cpu.al()));
                let segment = cpu.es;
                let value = runtime.memory.read_u8(segment, offset);
                let instruction_bytes: Vec<_> = (0..4)
                    .map(|index| runtime.memory.read_u8(cpu.cs, cpu.ip.wrapping_add(index)))
                    .collect();
                println!(
                    "XLAT before retired={} bytes={instruction_bytes:02x?} instruction=[26, d7] mode=386-real addr_size=16 operand_size=16 CPU={cpu:04x?} offset={offset:04x} segment=ES:{segment:04x} linear={:05x} value={value:02x}",
                    runtime.instructions_retired(),
                    chronos_core::GuestMemory::physical_address(segment, offset)
                );
                Some(cpu)
            } else {
                None
            };
            let cwd_before = if opcode == 0x66
                && runtime
                    .memory
                    .read_u8(runtime.cpu.cs, runtime.cpu.ip.wrapping_add(1))
                    == 0x99
            {
                let cpu = runtime.cpu;
                println!(
                    "CWD/CDQ before retired={} mode=386-real default_operand_size=16 override=66 operand_size=32 bytes=[66, 99] CS:IP={:04x}:{:04x} EAX={:08x} EDX={:08x} FLAGS={:04x}",
                    runtime.instructions_retired(), cpu.cs, cpu.ip, cpu.reg32(0), cpu.reg32(2), cpu.flags
                );
                Some(cpu)
            } else {
                None
            };
            let aam_before = if opcode == 0xd4 {
                let cpu = runtime.cpu;
                let base = runtime.memory.read_u8(cpu.cs, cpu.ip.wrapping_add(1));
                println!(
                    "AAM before retired={} bytes=[d4, {base:02x}] mode=386-real default_operand_size=16 operand_size=8 CS:IP={:04x}:{:04x} AL={:02x} AH={:02x} FLAGS={:04x}",
                    runtime.instructions_retired(), cpu.cs, cpu.ip, cpu.al(), cpu.ah(), cpu.flags
                );
                Some(cpu)
            } else {
                None
            };
            let disk_space = opcode == 0xcd && interrupt == 0x21 && runtime.cpu.ah() == 0x36;
            if disk_space {
                let drives = runtime.drives_mut();
                println!(
                    "DOS disk space current={:?} C:={:?} D:={:?} T:={:?}",
                    drives.current_drive,
                    drives.drive_space(DosDrive::C),
                    drives.drive_space(DosDrive::D),
                    drives.drive_space(DosDrive::T)
                );
            }
            runtime.step();
            if disk_space {
                println!(
                    "DOS disk space after retired={} CPU={:04x?}",
                    runtime.instructions_retired(),
                    runtime.cpu
                );
            }
            if let Some(before) = cwd_before {
                println!(
                    "CWD/CDQ after retired={} CS:IP={:04x}:{:04x} EAX={:08x} EDX={:08x} FLAGS={:04x} before_CS:IP={:04x}:{:04x} before_EAX={:08x} before_EDX={:08x} before_FLAGS={:04x} state={:?}",
                    runtime.instructions_retired(), runtime.cpu.cs, runtime.cpu.ip,
                    runtime.cpu.reg32(0), runtime.cpu.reg32(2), runtime.cpu.flags,
                    before.cs, before.ip, before.reg32(0), before.reg32(2), before.flags,
                    runtime.state()
                );
            }
            if let Some(before) = aam_before {
                println!(
                    "AAM after retired={} CS:IP={:04x}:{:04x} AL={:02x} AH={:02x} FLAGS={:04x} before_CS:IP={:04x}:{:04x} before_AL={:02x} before_AH={:02x} before_FLAGS={:04x} state={:?}",
                    runtime.instructions_retired(), runtime.cpu.cs, runtime.cpu.ip,
                    runtime.cpu.al(), runtime.cpu.ah(), runtime.cpu.flags,
                    before.cs, before.ip, before.al(), before.ah(), before.flags,
                    runtime.state()
                );
            }
            if let Some(before) = xlat_before {
                println!(
                    "XLAT after retired={} IP={:04x}:{:04x} EAX={:08x} EFLAGS={:04x} before_IP={:04x} before_EAX={:08x} before_FLAGS={:04x} state={:?}",
                    runtime.instructions_retired(), runtime.cpu.cs, runtime.cpu.ip,
                    runtime.cpu.reg32(0), runtime.cpu.flags, before.ip, before.reg32(0),
                    before.flags, runtime.state()
                );
            }
            if let Some((retired, before, port)) = io_before {
                println!(
                    "io retired={retired} {:04x}:{:04x} opcode={opcode:02x} port={port:04x} AX={:04x}->{:04x} DX={:04x} state={:?}",
                    before.cs, before.ip, before.ax, runtime.cpu.ax, before.dx, runtime.state()
                );
            }
            if multiplex_iret {
                println!(
                    "after multiplex IRET retired={} CPU={:04x?}",
                    runtime.instructions_retired(),
                    runtime.cpu
                );
            }
            if opcode == 0xcd && interrupt == 0x10 {
                dump_video(&runtime, "after");
            }
            if opcode == 0xc0 || opcode == 0xc1 {
                println!(
                    "after shift state={:?} {:04x?}",
                    runtime.state(),
                    runtime.cpu
                );
            }
            if opcode == 0xcd && interrupt == 0x21 && runtime.cpu.ah() == 0x38 {
                let mut buffer = [0; 34];
                runtime
                    .memory
                    .read_slice(runtime.cpu.ds, runtime.cpu.dx, &mut buffer)
                    .unwrap();
                println!("country buffer DS:DX={:04x}:{:04x} physical={:05x} length=34 bytes={buffer:02x?}", runtime.cpu.ds, runtime.cpu.dx, chronos_core::GuestMemory::physical_address(runtime.cpu.ds, runtime.cpu.dx));
            }
            if opcode == 0xcd {
                println!(
                    "after INT {interrupt:02x} state={:?} {:04x?}",
                    runtime.state(),
                    runtime.cpu
                );
            }
        } else {
            runtime.run_slice(10_000);
        }
        recovered_child_trap = runtime.take_recovered_child_trap();
        if recovered_child_trap.is_some() {
            break;
        }
        match runtime.state() {
            GuestState::Running | GuestState::Ready => {}
            GuestState::YieldedUntilTimer => {
                runtime.wake_from_timer();
            }
            _ => break,
        }
    }
    println!(
        "state={:?} retired={} CS:IP={:04x}:{:04x}",
        runtime.state(),
        runtime.instructions_retired(),
        runtime.cpu.cs,
        runtime.cpu.ip
    );
    if let Some(trap) = &recovered_child_trap {
        println!("recovered child trap={trap:?}");
    }
    if trace_system {
        println!(
            "IVT 15 observed states={system_vector_changes} last={previous_system_vector:04x?}"
        );
    }
    for interrupt in [0x15u16, 0x2f].into_iter().chain(0x34u16..=0x3f) {
        let offset = runtime.memory.read_u16(0, interrupt * 4);
        let segment = runtime.memory.read_u16(0, interrupt * 4 + 2);
        println!("INT {interrupt:02x} vector={segment:04x}:{offset:04x}");
    }
    for row in 0..25 {
        let line: String = (0..80)
            .map(|col| {
                let byte = runtime.cell(col, row).character;
                if byte.is_ascii_graphic() || byte == b' ' {
                    byte as char
                } else {
                    '.'
                }
            })
            .collect();
        if !line.trim().is_empty() {
            println!("{}", line.trim_end());
        }
    }
    println!("final PSP={:04x}", runtime.current_psp());
    if let Some(path) = env::var_os("CHRONOS_TEXT_CAPTURE") {
        capture_text(&runtime, Path::new(&path))?;
    }
    match runtime.state() {
        GuestState::Exited { code: 0 } if recovered_child_trap.is_none() => Ok(()),
        _ => process::exit(2),
    }
}

// Render the final guest text cells with the Chronos BIOS font. This is a
// host diagnostic image, separate from the native Chronos UI renderer.
fn capture_text(runtime: &Runtime, path: &Path) -> std::io::Result<()> {
    let font = include_bytes!("../fonts/cp437-16.bin");
    let mut raster = b"P6\n640 400\n255\n".to_vec();
    let rgb = |index: u8| {
        let channel = |bit| {
            let base = if index & bit != 0 { 170 } else { 0 };
            base + if index & 8 != 0 { 85 } else { 0 }
        };
        [channel(4), if index == 6 { 85 } else { channel(2) }, channel(1)]
    };
    for y in 0..400 {
        for col in 0..80 {
            let cell = runtime.cell(col, y / 16);
            let attr = runtime.text_attribute(cell.attribute);
            let glyph = font[usize::from(cell.character) * 16 + y % 16];
            for bit in (0..8).rev() {
                let color = if glyph & (1 << bit) != 0 {
                    attr.foreground
                } else {
                    attr.background
                };
                raster.extend_from_slice(&rgb(color));
            }
        }
    }
    fs::write(path, raster)
}

fn dump_video(runtime: &Runtime, label: &str) {
    let bytes: Vec<_> = (0..8)
        .map(|i| {
            runtime
                .memory
                .read_u8(runtime.cpu.cs, runtime.cpu.ip.wrapping_add(i))
        })
        .collect();
    let vectors: Vec<_> = [0x10u16, 0x1f, 0x43]
        .iter()
        .map(|v| {
            (
                v,
                runtime.memory.read_u16(0, v * 4 + 2),
                runtime.memory.read_u16(0, v * 4),
            )
        })
        .collect();
    let font: Vec<_> = (0..16)
        .map(|i| {
            runtime
                .memory
                .read_u8(runtime.cpu.es, runtime.cpu.bp.wrapping_add(i))
        })
        .collect();
    println!("{label} bytes={bytes:02x?} IVT(number,segment,offset)={vectors:04x?} ES:BP bytes={font:02x?}");
    if runtime.cpu.ah() == 0x12 && runtime.cpu.bl() == 0x32 {
        println!(
            "{label} VGA decode={} DAC generation={} checksum={:016x} framebuffer generation={}",
            runtime.memory.vga_addressing_enabled(),
            runtime.palette_generation(),
            runtime.palette_checksum(),
            runtime.framebuffer_generation(),
        );
    }
    if runtime.cpu.ah() == 0x1c {
        let physical = chronos_core::GuestMemory::physical_address(runtime.cpu.es, runtime.cpu.bx);
        let buffer: Vec<_> = (0..0x90)
            .map(|offset| {
                runtime
                    .memory
                    .read_u8(runtime.cpu.es, runtime.cpu.bx.wrapping_add(offset))
            })
            .collect();
        println!(
            "{label} INT10/AH=1C AL={:02x} CX={:04x} ES:BX={:04x}:{:04x} physical={physical:05x} buffer={buffer:02x?}",
            runtime.cpu.al(),
            runtime.cpu.cx,
            runtime.cpu.es,
            runtime.cpu.bx
        );
    }
    let bda: Vec<_> = (0x49..=0x89)
        .map(|offset| runtime.memory.read_u8(0x40, offset))
        .collect();
    let text: Vec<_> = (0..32)
        .map(|offset| runtime.memory.read_u8(0xb800, offset))
        .collect();
    println!("{label} video mode={:?} attr_mode={:?} attr_generation={} cursor=({}, {}) shape={:04x} BDA[0040:0049..0089]={bda:02x?} B800:0000={text:02x?}", runtime.video_mode(), runtime.text_attribute_mode(), runtime.text_attribute_generation(), runtime.cursor_column(), runtime.cursor_row(), runtime.cursor_shape());
}

fn dump_arena(runtime: &Runtime) {
    let mut mcb = 0x0fffu16;
    for _ in 0..1024 {
        let kind = runtime.memory.read_u8(mcb, 0);
        let owner = runtime.memory.read_u16(mcb, 1);
        let size = runtime.memory.read_u16(mcb, 3);
        println!("MCB={mcb:04x} type={kind:02x} owner={owner:04x} paragraphs={size:04x}");
        if kind != b'M' {
            break;
        }
        let Some(next) = mcb.checked_add(size).and_then(|v| v.checked_add(1)) else {
            break;
        };
        mcb = next;
    }
}
