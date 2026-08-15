//! audioctl — CLI for audiod (`audio.v1`).

#![no_std]
#![no_main]

use core::fmt::Write;
use sunlight_audio::SystemSound;
use sunlight_audiod::{AudioClient, AudioClientError, DEFAULT_TONE_HZ, DEFAULT_TONE_MS};
use sunlight_ipc::ProcessExit;

const MAX_ARGS: usize = 16;

fn stdout_write(s: &str) {
    let mut data = s.as_bytes();
    while !data.is_empty() {
        match sunlight_libc::write(sunlight_libc::STDOUT, data) {
            Ok(n) if n > 0 => data = &data[n..],
            _ => break,
        }
    }
}

macro_rules! println {
    ($($arg:tt)*) => {{
        let mut buf = heapless::String::<512>::new();
        let _ = write!(&mut buf, $($arg)*);
        stdout_write(&buf);
        stdout_write("\n");
    }};
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    println!("audioctl: PANIC");
    ProcessExit::exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let args = &storage[..count];
    let client = AudioClient::new();
    let sub = args.get(1).copied().unwrap_or("status");
    let code = match sub {
        "status" => match client.snapshot() {
            Ok(snap) => {
                println!("device: {}", snap.device_name());
                println!("state: {}", snap.state_label());
                println!("volume: {}%", snap.volume);
                println!("muted: {}", if snap.muted { "yes" } else { "no" });
                if let Some(fmt) = snap.format() {
                    println!(
                        "format: {} Hz {}-bit {}ch",
                        fmt.sample_rate_hz, fmt.bits_per_sample, fmt.channels
                    );
                }
                if snap.vendor_id != 0 {
                    println!("pci: {:04x}:{:04x}", snap.vendor_id, snap.device_id);
                }
                println!("underruns: {}", snap.underruns);
                println!("frames: {}", snap.frames_played);
                println!(
                    "system sounds: {}",
                    if snap.system_sounds_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
                println!("system sounds volume: {}%", snap.system_sounds_volume);
                println!("system sound queue: {}", snap.system_sound_queue_len);
                0
            }
            Err(err) => {
                println!("audioctl: {}", error_label(err));
                1
            }
        },
        "volume" => {
            if args.len() >= 3 {
                match parse_u8(args[2]) {
                    Some(v) if v <= 100 => match client.set_volume(v) {
                        Ok(snap) => {
                            println!("volume: {}%", snap.volume);
                            0
                        }
                        Err(err) => {
                            println!("audioctl: {}", error_label(err));
                            1
                        }
                    },
                    _ => {
                        println!("Usage: audioctl volume [0-100]");
                        1
                    }
                }
            } else {
                match client.snapshot() {
                    Ok(snap) => {
                        println!("{}", snap.volume);
                        0
                    }
                    Err(err) => {
                        println!("audioctl: {}", error_label(err));
                        1
                    }
                }
            }
        }
        "mute" => match client.set_mute(true) {
            Ok(_) => {
                println!("muted");
                0
            }
            Err(err) => {
                println!("audioctl: {}", error_label(err));
                1
            }
        },
        "unmute" => match client.set_mute(false) {
            Ok(_) => {
                println!("unmuted");
                0
            }
            Err(err) => {
                println!("audioctl: {}", error_label(err));
                1
            }
        },
        "test" => match client.play_tone(DEFAULT_TONE_HZ, DEFAULT_TONE_MS) {
            Ok(()) => {
                println!("playing {} Hz for {} ms", DEFAULT_TONE_HZ, DEFAULT_TONE_MS);
                0
            }
            Err(err) => {
                println!("audioctl: {}", error_label(err));
                1
            }
        },
        "system-on" => match client.set_system_sounds_enabled(true) {
            Ok(_) => {
                println!("system sounds enabled");
                0
            }
            Err(err) => {
                println!("audioctl: {}", error_label(err));
                1
            }
        },
        "system-off" => match client.set_system_sounds_enabled(false) {
            Ok(_) => {
                println!("system sounds disabled");
                0
            }
            Err(err) => {
                println!("audioctl: {}", error_label(err));
                1
            }
        },
        "system-volume" => match args.get(2).and_then(|value| parse_u8(value)) {
            Some(value) if value <= 100 => match client.set_system_sounds_volume(value) {
                Ok(snapshot) => {
                    println!("system sounds volume: {}%", snapshot.system_sounds_volume);
                    0
                }
                Err(err) => {
                    println!("audioctl: {}", error_label(err));
                    1
                }
            },
            _ => {
                println!("Usage: audioctl system-volume [0-100]");
                1
            }
        },
        "preview" => match args.get(2).and_then(|name| parse_system_sound(name)) {
            Some(sound) => match client.preview_system_sound(sound) {
                Ok(()) => {
                    println!("previewing {}", sound.label());
                    0
                }
                Err(err) => {
                    println!("audioctl: {}", error_label(err));
                    1
                }
            },
            None => {
                println!(
                    "Usage: audioctl preview <notification|message|success|warning|error|question|critical|device-connected|device-disconnected|volume-changed>"
                );
                1
            }
        },
        _ => {
            println!(
                "Usage: audioctl <status|volume|mute|unmute|test|system-on|system-off|system-volume|preview>"
            );
            println!("  audioctl status");
            println!("  audioctl volume");
            println!("  audioctl volume 50");
            println!("  audioctl mute");
            println!("  audioctl unmute");
            println!("  audioctl test");
            println!("  audioctl system-on");
            println!("  audioctl system-off");
            println!("  audioctl system-volume 60");
            println!("  audioctl preview warning");
            1
        }
    };
    ProcessExit::exit(code);
}

fn parse_system_sound(name: &str) -> Option<SystemSound> {
    match name {
        "notification" => Some(SystemSound::Notification),
        "message" => Some(SystemSound::Message),
        "success" => Some(SystemSound::Success),
        "warning" => Some(SystemSound::Warning),
        "error" => Some(SystemSound::Error),
        "question" => Some(SystemSound::Question),
        "critical" => Some(SystemSound::Critical),
        "device-connected" => Some(SystemSound::DeviceConnected),
        "device-disconnected" => Some(SystemSound::DeviceDisconnected),
        "volume-changed" => Some(SystemSound::VolumeChanged),
        _ => None,
    }
}

fn error_label(err: AudioClientError) -> &'static str {
    match err {
        AudioClientError::ServiceUnavailable => "audiod not running",
        AudioClientError::Timeout => "timeout",
        AudioClientError::Transport => "ipc failure",
        AudioClientError::Unavailable => "no audio output device available",
        AudioClientError::BadRequest => "bad request",
        AudioClientError::InvalidFormat => "invalid sample format",
        AudioClientError::Overflow => "buffer too large",
        AudioClientError::DeviceFailed => "device failed",
    }
}

fn parse_u8(s: &str) -> Option<u8> {
    let mut n: u16 = 0;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.saturating_mul(10).saturating_add((b - b'0') as u16);
        if n > 255 {
            return Some(255);
        }
    }
    Some(n as u8)
}

unsafe fn collect_args<'a>(argc: u64, argv: *const *const u8, out: &mut [&'a str]) -> usize {
    let n = (argc as usize).min(out.len());
    let mut count = 0;
    let mut i = 0;
    while i < n {
        let ptr = unsafe { *argv.add(i) };
        if ptr.is_null() {
            break;
        }
        let mut len = 0usize;
        while unsafe { *ptr.add(len) } != 0 && len < 64 {
            len += 1;
        }
        out[count] =
            unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len)) };
        count += 1;
        i += 1;
    }
    count
}
