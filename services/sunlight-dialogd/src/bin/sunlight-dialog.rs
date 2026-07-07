#![no_std]
#![no_main]
#![allow(static_mut_refs)]

extern crate alloc;

use alloc::string::ToString;
use alloc::vec::Vec;

use sunlight_dialogs::{
    decode_result, encode_request, AlertRequest, ConfirmRequest, ConfirmStyle, DialogButton,
    DialogCommonOptions, DialogError, DialogMsg, DialogRequest, DialogResult, OpenFileRequest,
    OpenFolderRequest, SaveFileRequest, TextInputRequest,
};
use sunlight_ipc::{
    ipc_call, nameserver_lookup, nameserver_lookup_timeout, process_yield, shm_alloc, shm_free,
    shm_map, CapabilityToken, IpcMsg, ProcessExit, SHM_PAGE,
};
use sunlight_libc as libc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 160 * 1024] = [0; 160 * 1024];
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

const MAX_ARGS: usize = 32;

macro_rules! println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<512>::new();
        let _ = write!(&mut buf, $($arg)*);
        stdout_write(&buf);
        stdout_write("\n");
    }};
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let args = &storage[..count];

    let Some(cap) = ensure_dialog_service() else {
        println!("dialog host unavailable");
        ProcessExit::exit(1);
    };

    let code = match parse_request(args) {
        Ok(request) => run_request(cap, request),
        Err(message) => {
            println!("{}", message);
            print_usage();
            1
        }
    };

    ProcessExit::exit(code);
}

fn parse_request(args: &[&str]) -> Result<DialogRequest, &'static str> {
    let Some(cmd) = args.get(1).copied() else {
        return Err("missing subcommand");
    };
    match cmd {
        "alert" => {
            let title = read_flag(args, "--title").unwrap_or("");
            let message = read_flag(args, "--message").unwrap_or("");
            Ok(DialogRequest::Alert(AlertRequest {
                common: DialogCommonOptions {
                    title: title.to_string(),
                    message: message.to_string(),
                },
            }))
        }
        "confirm" => {
            let title = read_flag(args, "--title").unwrap_or("");
            let message = read_flag(args, "--message").unwrap_or("");
            let style = match read_flag(args, "--style").unwrap_or("yes-no") {
                "ok-cancel" => ConfirmStyle::OkCancel,
                _ => ConfirmStyle::YesNo,
            };
            Ok(DialogRequest::Confirm(ConfirmRequest {
                common: DialogCommonOptions {
                    title: title.to_string(),
                    message: message.to_string(),
                },
                style,
                default_button: match style {
                    ConfirmStyle::OkCancel => DialogButton::Ok,
                    ConfirmStyle::YesNo => DialogButton::Yes,
                },
            }))
        }
        "input" => {
            let title = read_flag(args, "--title").unwrap_or("");
            let message = read_flag(args, "--message").unwrap_or("");
            let default_value = read_flag(args, "--default").unwrap_or("");
            let allow_empty = !has_flag(args, "--no-empty");
            Ok(DialogRequest::TextInput(TextInputRequest {
                common: DialogCommonOptions {
                    title: title.to_string(),
                    message: message.to_string(),
                },
                default_value: default_value.to_string(),
                allow_empty,
            }))
        }
        "open-file" => Ok(DialogRequest::OpenFile(OpenFileRequest {
            title: read_flag(args, "--title")
                .unwrap_or("Open File")
                .to_string(),
            initial_dir: read_flag(args, "--initial-dir").map(StringLike::to_string_like),
            allowed_mime_types: read_csv_flag(args, "--mime"),
            allowed_extensions: read_csv_flag(args, "--ext"),
            allow_multiple: has_flag(args, "--multiple"),
            show_preview: !has_flag(args, "--no-preview"),
            confirm_button_label: read_flag(args, "--confirm").map(StringLike::to_string_like),
        })),
        "open-folder" => Ok(DialogRequest::OpenFolder(OpenFolderRequest {
            title: read_flag(args, "--title")
                .unwrap_or("Open Folder")
                .to_string(),
            initial_dir: read_flag(args, "--initial-dir").map(StringLike::to_string_like),
            confirm_button_label: read_flag(args, "--confirm").map(StringLike::to_string_like),
        })),
        "save-file" => Ok(DialogRequest::SaveFile(SaveFileRequest {
            title: read_flag(args, "--title")
                .unwrap_or("Save File")
                .to_string(),
            initial_dir: read_flag(args, "--initial-dir").map(StringLike::to_string_like),
            suggested_name: read_flag(args, "--suggested-name").map(StringLike::to_string_like),
            default_extension: read_flag(args, "--default-extension")
                .map(StringLike::to_string_like),
            allowed_extensions: read_csv_flag(args, "--ext"),
            overwrite_confirm: !has_flag(args, "--no-overwrite-confirm"),
            confirm_button_label: read_flag(args, "--confirm").map(StringLike::to_string_like),
        })),
        _ => Err("unknown subcommand"),
    }
}

fn run_request(cap: CapabilityToken, request: DialogRequest) -> i32 {
    let body = encode_request(&request);
    let reply = match call_with_page(cap, DialogMsg::SHOW_DIALOG, &body) {
        Ok(reply) => reply,
        Err(err) => return print_error(err),
    };
    if reply.label == DialogMsg::ERROR {
        return print_error(DialogError::from_code(reply.words[0]));
    }
    let bytes = match take_reply_bytes(&reply) {
        Ok(bytes) => bytes,
        Err(err) => return print_error(err),
    };
    let result = match decode_result(&bytes) {
        Ok(result) => result,
        Err(err) => return print_error(err),
    };
    match result {
        DialogResult::Ok => println!("ok"),
        DialogResult::Cancel => println!("cancel"),
        DialogResult::Yes => println!("yes"),
        DialogResult::No => println!("no"),
        DialogResult::Dismissed => println!("dismissed"),
        DialogResult::TextSubmitted(text) => println!("{}", text),
        DialogResult::FileSelected(path) => println!("{}", path),
        DialogResult::FilesSelected(paths) => {
            for path in paths {
                println!("{}", path);
            }
        }
        DialogResult::FolderSelected(path) => println!("{}", path),
        DialogResult::SavePathSelected(path) => println!("{}", path),
        DialogResult::Cancelled => println!("cancelled"),
        DialogResult::Error(message) => println!("error: {}", message),
    }
    0
}

fn call_with_page(cap: CapabilityToken, label: u64, body: &[u8]) -> Result<IpcMsg, DialogError> {
    if body.len() > SHM_PAGE {
        return Err(DialogError::TooLarge);
    }
    let (ptr, token) = shm_alloc().map_err(|_| DialogError::Internal)?;
    unsafe {
        core::ptr::copy_nonoverlapping(body.as_ptr(), ptr, body.len());
    }
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(label)
            .word(0, body.len() as u64)
            .with_cap(0, token),
    );
    let _ = shm_free(token);
    Ok(reply)
}

fn take_reply_bytes(reply: &IpcMsg) -> Result<Vec<u8>, DialogError> {
    let len = reply.words[1] as usize;
    let token = reply.caps[0];
    if len == 0 || len > SHM_PAGE || token == CapabilityToken::INVALID {
        return Err(DialogError::Corrupt);
    }
    let ptr = shm_map(token).map_err(|_| DialogError::Corrupt)?;
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
    let _ = shm_free(token);
    Ok(bytes)
}

fn ensure_dialog_service() -> Option<CapabilityToken> {
    if let Some(cap) = nameserver_lookup("dialogd") {
        return Some(cap);
    }
    if let Some(cap) = nameserver_lookup_timeout("dialogd", 50) {
        return Some(cap);
    }
    let _ = libc::spawn(b"/sbin/sunlight-dialogd", &[b"sunlight-dialogd"], None)
        .or_else(|_| libc::spawn(b"/bin/sunlight-dialogd", &[b"sunlight-dialogd"], None));
    for _ in 0..8 {
        if let Some(cap) = nameserver_lookup_timeout("dialogd", 75) {
            return Some(cap);
        }
        process_yield();
    }
    None
}

fn read_flag<'a>(args: &'a [&str], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find_map(|pair| (pair[0] == flag).then_some(pair[1]))
}

fn read_csv_flag(args: &[&str], flag: &str) -> Vec<alloc::string::String> {
    let mut out = Vec::new();
    if let Some(value) = read_flag(args, flag) {
        for part in value.split(',') {
            if !part.is_empty() {
                out.push(part.to_string());
            }
        }
    }
    out
}

fn has_flag(args: &[&str], flag: &str) -> bool {
    args.iter().any(|arg| *arg == flag)
}

trait StringLike {
    fn to_string_like(self) -> alloc::string::String;
}

impl StringLike for &str {
    fn to_string_like(self) -> alloc::string::String {
        self.to_string()
    }
}

fn stdout_write(s: &str) {
    let mut data = s.as_bytes();
    while !data.is_empty() {
        match sunlight_libc::write(sunlight_libc::STDOUT, data) {
            Ok(n) if n > 0 => data = &data[n..],
            _ => break,
        }
    }
}

fn print_usage() {
    println!("Usage:");
    println!("  sunlight-dialog alert --title <title> --message <message>");
    println!(
        "  sunlight-dialog confirm --title <title> --message <message> [--style yes-no|ok-cancel]"
    );
    println!("  sunlight-dialog input --title <title> --message <message> [--default <text>] [--no-empty]");
    println!("  sunlight-dialog open-file [--title <title>] [--initial-dir <dir>] [--ext txt,rs] [--mime text/plain]");
    println!("  sunlight-dialog open-folder [--title <title>] [--initial-dir <dir>]");
    println!("  sunlight-dialog save-file [--title <title>] [--initial-dir <dir>] [--suggested-name <name>]");
}

fn print_error(err: DialogError) -> i32 {
    match err {
        DialogError::BadRequest => println!("bad request"),
        DialogError::TooLarge => println!("dialog payload too large"),
        DialogError::Unsupported => println!("dialog type not implemented yet"),
        DialogError::Busy => println!("dialog host busy"),
        DialogError::HostUnavailable => println!("dialog host unavailable"),
        DialogError::Corrupt => println!("corrupt dialog reply"),
        DialogError::Internal => println!("dialog error"),
    }
    1
}

unsafe fn collect_args(argc: u64, argv: *const *const u8, out: &mut [&str]) -> usize {
    let mut count = 0usize;
    for index in 0..(argc as usize).min(out.len()) {
        let ptr = *argv.add(index);
        if ptr.is_null() {
            break;
        }
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        out[count] = core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len));
        count += 1;
    }
    count
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    println!("sunlight-dialog: PANIC");
    ProcessExit::exit(101);
}
