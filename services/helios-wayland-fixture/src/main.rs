#![no_std]
#![no_main]

use core::fmt::{self, Write};
use sunlight_ipc::{
    debug_log, monotonic_millis, process_yield, sgp::SgpMsg, ProcessExit, SunlightSyscall,
};
use sunlight_libc::{close, mman, sys, write, Fd};
use sunlight_ui::{theme::Color, Window, WindowConfig, WindowDecoration};

mod desktop;

const WIDTH: usize = 256;
const HEIGHT: usize = 160;
const TITLE: &str = "HeliosWL";
const GLOBALS: &[&[u8]] = &[
    b"wl_compositor",
    b"wl_shm",
    b"xdg_wm_base",
    #[cfg(feature = "pointer-fixture")]
    b"wl_seat",
];

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[WL-FIXTURE] panic\n");
    ProcessExit::exit(1)
}

struct Log {
    bytes: [u8; 128],
    len: usize,
}

impl Write for Log {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let end = self.len + value.len();
        self.bytes
            .get_mut(self.len..end)
            .ok_or(fmt::Error)?
            .copy_from_slice(value.as_bytes());
        self.len = end;
        Ok(())
    }
}

fn log(args: fmt::Arguments<'_>) {
    let mut log = Log {
        bytes: [0; 128],
        len: 0,
    };
    if log.write_fmt(args).is_ok() {
        debug_log(core::str::from_utf8(&log.bytes[..log.len]).unwrap());
    }
}

struct Descriptor(i32);
impl Drop for Descriptor {
    fn drop(&mut self) {
        let _ = close(Fd(self.0 as u32));
    }
}

struct Message {
    bytes: [u8; 128],
    len: usize,
    right: Option<(usize, Descriptor)>,
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn receive(stream: &Descriptor) -> Result<Option<Message>, &'static str> {
    let mut msg = Message {
        bytes: [0; 128],
        len: 0,
        right: None,
    };
    let mut target = 8;
    while msg.len < target {
        let result = unsafe {
            sys::syscall3(
                sys::SYS_READ,
                stream.0 as u64,
                msg.bytes[msg.len..].as_mut_ptr() as u64,
                (target - msg.len) as u64,
            )
        } as i64;
        match result {
            0 if msg.len == 0 => return Ok(None),
            0 => return Err("partial request at EOF"),
            -2 => process_yield(),
            -95 => {
                let right = unsafe {
                    sys::syscall1(SunlightSyscall::NativeUnixRecvFd as u64, stream.0 as u64)
                };
                if (right as i64) < 0 {
                    return Err("SCM_RIGHTS receive");
                }
                let fd = Descriptor(right as i32);
                let byte = (right >> 32) as u8;
                if msg.right.is_some() {
                    return Err("second FD in request");
                }
                msg.right = Some((msg.len, fd));
                msg.bytes[msg.len] = byte;
                msg.len += 1;
            }
            count if count > 0 && count as usize <= target - msg.len => msg.len += count as usize,
            _ => return Err("stream read"),
        }
        if msg.len == 8 {
            target = (word(&msg.bytes, 4) >> 16) as usize;
            if !(8..=msg.bytes.len()).contains(&target) || target % 4 != 0 {
                return Err("request size");
            }
        }
    }
    Ok(Some(msg))
}

fn event(stream: &Descriptor, id: u32, opcode: u32, args: &[u32]) -> Result<(), &'static str> {
    let len = 8 + args.len() * 4;
    let mut bytes = [0; 64];
    for (chunk, value) in bytes.chunks_exact_mut(4).zip(
        [id, ((len as u32) << 16) | opcode]
            .into_iter()
            .chain(args.iter().copied()),
    ) {
        chunk.copy_from_slice(&value.to_le_bytes());
    }
    let mut sent = 0;
    while sent < len {
        let count = write(Fd(stream.0 as u32), &bytes[sent..len]).map_err(|_| "event write")?;
        if count == 0 {
            return Err("event write zero");
        }
        sent += count;
    }
    Ok(())
}

fn advertise(stream: &Descriptor) -> Result<(), &'static str> {
    for (index, interface) in GLOBALS.iter().enumerate() {
        let mut args = [0; 8];
        args[0] = index as u32 + 1;
        args[1] = interface.len() as u32 + 1;
        for (offset, byte) in interface.iter().enumerate() {
            args[2 + offset / 4] |= (*byte as u32) << (8 * (offset % 4));
        }
        let version = 2 + (interface.len() + 1).div_ceil(4);
        args[version] = 1;
        event(stream, 2, 0, &args[..=version])?;
    }
    Ok(())
}

struct Pool {
    addr: *mut u8,
    len: usize,
}

impl Pool {
    fn map(fd: Descriptor, len: usize) -> Result<Self, &'static str> {
        if !(1..=1024 * 1024).contains(&len) {
            return Err("invalid pool size");
        }
        let addr = unsafe {
            sys::syscall6(
                sys::SYS_MMAP,
                0,
                len as u64,
                mman::PROT_READ as u64,
                mman::MAP_SHARED as u64,
                fd.0 as u64,
                0,
            )
        };
        if addr == 0 || (addr as i64) < 0 {
            return Err("pool exceeds backing or cannot be mapped");
        }
        Ok(Self {
            addr: addr as *mut u8,
            len,
        })
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        if mman::munmap(self.addr, self.len).is_err() {
            debug_log("[WL-FIXTURE] unmap failed\n");
        }
    }
}

#[derive(Clone, Copy)]
struct Buffer {
    id: u32,
    layout: Option<(usize, usize)>,
}

fn layout(bytes: &[u8], pool_len: usize) -> Option<(usize, usize)> {
    let offset = usize::try_from(word(bytes, 12) as i32).ok()?;
    let stride = usize::try_from(word(bytes, 24) as i32).ok()?;
    let row = WIDTH * 4;
    if word(bytes, 16) != WIDTH as u32
        || word(bytes, 20) != HEIGHT as u32
        || word(bytes, 28) != 1
        || stride < row
    {
        return None;
    }
    let end = offset
        .checked_add((HEIGHT - 1).checked_mul(stride)?)?
        .checked_add(row)?;
    (end <= pool_len).then_some((offset, stride))
}

fn claim_object(objects: &mut u32, id: u32) -> Result<u32, &'static str> {
    if !(2..32).contains(&id) || *objects & (1 << id) != 0 {
        return Err("invalid object ID");
    }
    *objects |= 1 << id;
    Ok(id)
}

fn serve(stream: &Descriptor) -> Result<(), &'static str> {
    let mut registry = false;
    let mut synced = false;
    let mut objects = 0;
    let mut bound = [0; GLOBALS.len()];
    #[cfg(feature = "pointer-fixture")]
    let mut pointer_id = 0;
    #[cfg(feature = "pointer-fixture")]
    let mut click_sent = false;
    let mut surface_id = 0;
    let mut xdg_id = 0;
    let mut toplevel_id = 0;
    let mut pool_id = 0;
    let mut role = 0;
    let mut configured = false;
    let mut acked = false;
    let mut pool = None;
    let mut buffers = [None::<Buffer>; 3];
    let mut pending = None;
    let mut callback = None;
    let mut surface = None::<Window>;
    let mut frame = 0;
    let result = (|| {
        while let Some(mut msg) = receive(stream)? {
            let bytes = &msg.bytes[..msg.len];
            let id = word(bytes, 0);
            let opcode = word(bytes, 4) & 0xffff;
            if msg.right.is_some() && (id, opcode) != (bound[1], 0) {
                return Err("FD outside create_pool");
            }
            match (id, opcode, msg.len) {
                (1, 1, 12) if !registry && word(bytes, 8) == 2 => {
                    claim_object(&mut objects, 2)?;
                    advertise(stream)?;
                    debug_log("[WL-FIXTURE] advertised globals registry=2\n");
                    registry = true;
                }
                (1, 0, 12) if registry && !synced && bound == [0; GLOBALS.len()] => {
                    let sync_id = claim_object(&mut objects, word(bytes, 8))?;
                    event(stream, sync_id, 0, &[0])?;
                    event(stream, 1, 1, &[sync_id])?;
                    objects &= !(1 << sync_id);
                    synced = true;
                    log(format_args!("[WL-FIXTURE] registry sync callback={sync_id}\n"));
                }
                #[cfg(feature = "pointer-fixture")]
                (1, 0, 12) if frame == 1 && pointer_id != 0 && !click_sent => {
                    let sync_id = claim_object(&mut objects, word(bytes, 8))?;
                    let time = monotonic_millis() as u32;
                    event(stream, pointer_id, 0, &[2, surface_id, 64 << 8, 112 << 8])?;
                    event(stream, pointer_id, 3, &[3, time, 0x110, 1])?;
                    event(stream, pointer_id, 3, &[4, time, 0x110, 0])?;
                    event(stream, sync_id, 0, &[4])?;
                    event(stream, 1, 1, &[sync_id])?;
                    objects &= !(1 << sync_id);
                    click_sent = true;
                    log(format_args!(
                        "[WL-FIXTURE] click surface={surface_id} x=64 y=112 button=272 state=1 serial=3\n"
                    ));
                }
                (2, 0, len) if registry && len >= 24 => {
                    let index = word(bytes, 8).wrapping_sub(1) as usize;
                    let interface = *GLOBALS.get(index).ok_or("unknown global")?;
                    let version = 16 + ((interface.len() + 1).div_ceil(4) * 4);
                    if bound[index] != 0
                        || len != version + 8
                        || word(bytes, 12) as usize != interface.len() + 1
                        || &bytes[16..16 + interface.len()] != interface
                        || bytes[16 + interface.len()] != 0
                        || word(bytes, version) != 1
                    {
                        return Err("invalid registry bind");
                    }
                    bound[index] = claim_object(&mut objects, word(bytes, version + 4))?;
                    if index == 1 {
                        event(stream, bound[1], 0, &[0])?;
                        event(stream, bound[1], 0, &[1])?;
                    }
                    #[cfg(feature = "pointer-fixture")]
                    if index == 3 {
                        event(stream, bound[3], 0, &[1])?;
                    }
                }
                #[cfg(feature = "pointer-fixture")]
                (id, 0, 12) if id != 0 && id == bound[3] && pointer_id == 0 => {
                    pointer_id = claim_object(&mut objects, word(bytes, 8))?;
                }
                (id, 0, 12) if id != 0 && id == bound[0] && role == 0 => {
                    surface_id = claim_object(&mut objects, word(bytes, 8))?;
                    role = 1;
                }
                (id, 2, 16)
                    if id != 0 && id == bound[2] && role == 1 && word(bytes, 12) == surface_id =>
                {
                    xdg_id = claim_object(&mut objects, word(bytes, 8))?;
                    role = 2;
                }
                (id, 1, 12) if id == xdg_id && role == 2 => {
                    toplevel_id = claim_object(&mut objects, word(bytes, 8))?;
                    role = 3;
                }
                (id, 6, 8) if id == surface_id && role == 3 && !configured => {
                    event(stream, toplevel_id, 0, &[WIDTH as u32, HEIGHT as u32, 0])?;
                    event(stream, xdg_id, 0, &[1])?;
                    configured = true;
                    debug_log("[WL-FIXTURE] configure serial=1\n");
                }
                (id, 4, 12) if id == xdg_id && configured && !acked && word(bytes, 8) == 1 => {
                    acked = true;
                    debug_log("[WL-FIXTURE] ack_configure serial=1\n");
                }
                (id, 0, 16) if id != 0 && id == bound[1] && pool.is_none() => {
                    pool_id = claim_object(&mut objects, word(bytes, 8))?;
                    let (mark, fd) = msg.right.take().ok_or("missing pool FD")?;
                    let len = word(bytes, 12) as usize;
                    pool = Some(Pool::map(fd, len)?);
                    log(format_args!(
                        "[WL-FIXTURE] create_pool bytes={len} fd_mark={mark}\n"
                    ));
                }
                (id, 0, 32) if id == pool_id && pool.is_some() => {
                    let id = claim_object(&mut objects, word(bytes, 8))?;
                    let slot = buffers
                        .iter_mut()
                        .find(|slot| slot.is_none())
                        .ok_or("buffer limit")?;
                    *slot = Some(Buffer {
                        id,
                        layout: layout(bytes, pool.as_ref().unwrap().len),
                    });
                }
                (id, 1, 20)
                    if id == surface_id
                        && acked
                        && word(bytes, 12) == 0
                        && word(bytes, 16) == 0 =>
                {
                    pending = Some(
                        buffers
                            .iter()
                            .flatten()
                            .find(|buf| buf.id == word(bytes, 8))
                            .copied()
                            .ok_or("unknown attachment")?,
                    );
                }
                (id, 2, 24) if id == surface_id && acked => {}
                (id, 3, 12) if id == surface_id && acked && callback.is_none() => {
                    callback = Some(claim_object(&mut objects, word(bytes, 8))?);
                }
                (id, 6, 8) if id == surface_id && acked => {
                    if let Some(buffer) = pending.take() {
                        if let Some((offset, stride)) = buffer.layout {
                            if frame == 2 {
                                return Err("fixture frame limit");
                            }
                            if surface.is_none() {
                                if !desktop::wait_for_desktop(120_000) {
                                    return Err("desktop unavailable");
                                }
                                debug_log("[WL-FIXTURE] desktop ready for first commit\n");
                                surface = Some(
                                    Window::connect_with_flags(
                                        WindowConfig {
                                            width: WIDTH as u32,
                                            height: HEIGHT as u32,
                                            title: TITLE,
                                            decoration: WindowDecoration::Normal,
                                        },
                                        SgpMsg::config_flags::Z_ON_TOP,
                                    )
                                    .ok_or("SGP window creation")?,
                                );
                            }
                            let window = surface.as_mut().unwrap();
                            if frame == 0 && !window.set_close_grace(2000) {
                                return Err("SGP close grace");
                            }
                            let pool = pool.as_ref().unwrap();
                            let mut canvas = window.canvas();
                            let mut hash = 2166136261u32;
                            for y in 0..HEIGHT {
                                for x in 0..WIDTH {
                                    let addr =
                                        unsafe { pool.addr.add(offset + y * stride + x * 4) };
                                    let pixel = unsafe {
                                        u32::from_le_bytes(core::array::from_fn(|index| {
                                            addr.add(index).read_volatile()
                                        }))
                                    } | 0xff000000;
                                    hash = (hash ^ pixel).wrapping_mul(16777619);
                                    canvas.put_pixel(x as i32, y as i32, Color(pixel));
                                }
                            }
                            frame += 1;
                            log(format_args!("[WL-FIXTURE] wl_surface.commit frame={frame} buffer={} hash={hash:08x}\n", buffer.id));
                            window.commit();
                            log(format_args!(
                                "[WL-FIXTURE] SGP commit returned frame={frame}\n"
                            ));
                        } else {
                            log(format_args!(
                                "[WL-FIXTURE] skipped invalid buffer={}\n",
                                buffer.id
                            ));
                        }
                        event(stream, buffer.id, 0, &[])?;
                    }
                    if let Some(id) = callback.take() {
                        event(stream, id, 0, &[monotonic_millis() as u32])?;
                        event(stream, 1, 1, &[id])?;
                        objects &= !(1 << id);
                    }
                }
                _ => return Err("unsupported request or invalid order"),
            }
        }
        if frame == 2 {
            debug_log("[WL-FIXTURE] peer EOF after frame=2\n");
            Ok(())
        } else {
            Err("EOF before two frames")
        }
    })();
    drop(pool);
    if let Some(window) = surface {
        drop(window);
        debug_log("[WL-FIXTURE] SGP close returned\n");
    }
    result
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let name = b"/sunlight/native-unix/wayland-fixture";
    let listener = unsafe {
        sys::syscall3(
            SunlightSyscall::NativeUnixListen as u64,
            name.as_ptr() as u64,
            name.len() as u64,
            0,
        )
    } as i64;
    if listener < 0 {
        debug_log("[WL-FIXTURE] FAIL listen\n");
        ProcessExit::exit(1);
    }
    let listener = Descriptor(listener as i32);
    debug_log("[WL-FIXTURE] listener registered\n");
    let result = loop {
        let accepted =
            unsafe { sys::syscall1(SunlightSyscall::NativeUnixAccept as u64, listener.0 as u64) }
                as i64;
        if accepted == -11 {
            process_yield();
        } else if accepted >= 0 {
            let stream = Descriptor(accepted as i32);
            debug_log("[WL-FIXTURE] client accepted\n");
            break serve(&stream);
        } else {
            break Err("accept");
        }
    };
    drop(listener);
    match result {
        Ok(()) => debug_log("[WL-FIXTURE] complete after disconnect\n"),
        Err(error) => log(format_args!("[WL-FIXTURE] FAIL {error}\n")),
    }
    ProcessExit::exit(i32::from(result.is_err()))
}
