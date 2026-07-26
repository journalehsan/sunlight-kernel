use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

struct EmbeddedBinary<'a> {
    package: &'a str,
    output: &'a str,
    rustflags: &'a str,
    args: &'a [&'a str],
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing manifest dir"));
    let workspace_root = manifest_dir
        .parent()
        .expect("kernel should live one directory below workspace root");
    let target_dir = workspace_root.join("target");
    let release_dir = target_dir.join("x86_64-unknown-none").join("release");
    let scratch_target_dir = target_dir.join("embedded-build");

    let service_rustflags = "-C link-arg=-Tservices/user-space.ld -C relocation-model=static";
    let tls_rustflags = concat!(
        "-C link-arg=-Tservices/user-space.ld -C relocation-model=static ",
        "--cfg aes_force_soft ",
        "--cfg polyval_force_soft ",
        "--cfg poly1305_force_soft ",
        "--cfg chacha20_force_soft ",
        "--cfg curve25519_dalek_backend=\"serial\""
    );

    let binaries = [
        EmbeddedBinary {
            package: "sunlight-init",
            output: "sunlight-init",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-timer-server",
            output: "sunlight-timer-server",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-swapd",
            output: "sunlight-swapd",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-kbd",
            output: "sunlight-kbd",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-mouse",
            output: "sunlight-mouse",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-usb-mouse",
            output: "sunlight-usb-mouse",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-deviced",
            output: "deviced",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "deviced"],
        },
        EmbeddedBinary {
            package: "sunlight-networkd",
            output: "networkd",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "networkd"],
        },
        EmbeddedBinary {
            package: "sunlight-powerd",
            output: "powerd",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "powerd"],
        },
        EmbeddedBinary {
            package: "sunlight-thermald",
            output: "thermald",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "thermald"],
        },
        EmbeddedBinary {
            package: "sunlight-thermald",
            output: "thermalctl",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "thermalctl"],
        },
        // vfs-server (pulls in sunlight-fs which seeds /etc/locale.conf, /etc/locale.gen etc.)
        EmbeddedBinary {
            package: "sunlight-vfs-server",
            output: "sunlight-vfs-server",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-tty-server",
            output: "sunlight-tty-server",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-net-server",
            output: "net_server",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "timezone_service",
            output: "timezone_service",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-timed",
            output: "timed",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-tz",
            output: "tzutils",
            rustflags: service_rustflags,
            args: &["--release", "--features", "tzutils", "--bin", "tzutils"],
        },
        EmbeddedBinary {
            package: "sunlightd",
            output: "sunlightd",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "rand_service",
            output: "rand_service",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "secret_store_test",
            output: "secret_store_test",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        // sunshell (includes localectl builtin; transitively builds sunlight-locale etc.)
        EmbeddedBinary {
            package: "sunshell",
            output: "sshl",
            rustflags: service_rustflags,
            args: &[
                "--features",
                "sunlight",
                "--no-default-features",
                "--release",
            ],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "sunlight-utils",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "echo",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "echo"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "cat",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "cat"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "pwd",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "pwd"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "true",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "true"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "false",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "false"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "basename",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "basename"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "dirname",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "dirname"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "head",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "head"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "cmp",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "cmp"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "cksum",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "cksum"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "wc",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "wc"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "cut",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "cut"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "fold",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "fold"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "expand",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "expand"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "grep",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "grep"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "sort",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "sort"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "uniq",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "uniq"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "comm",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "comm"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "tr",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "tr"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "paste",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "paste"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "join",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "join"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "printf",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "printf"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "tee",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "tee"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "nl",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "nl"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "od",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "od"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "split",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "split"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "find",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "find"],
        },
        EmbeddedBinary {
            package: "sunlight-utils",
            output: "xargs",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "xargs"],
        },
        EmbeddedBinary {
            package: "sunlight-net-utils",
            output: "sunlight-net-utils",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-top",
            output: "sunlight-top",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-fetch",
            output: "fetch",
            rustflags: service_rustflags,
            args: &[
                "--features",
                "sunlightos",
                "--no-default-features",
                "--release",
            ],
        },
        EmbeddedBinary {
            package: "sunlightctl",
            output: "sunlightctl",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "mezzoctl",
            output: "mezzoctl",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-deviced",
            output: "devicectl",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "devicectl"],
        },
        EmbeddedBinary {
            package: "sunlight-deviced",
            output: "sunlight-hwinfo",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "sunlight-hwinfo"],
        },
        EmbeddedBinary {
            package: "sunlight-networkd",
            output: "networkctl",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "networkctl"],
        },
        EmbeddedBinary {
            package: "sunlight-powerd",
            output: "powerctl",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "powerctl"],
        },
        EmbeddedBinary {
            package: "sunlight-niced",
            output: "niced",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-gcd",
            output: "gcd",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-kv",
            output: "sunlight-kv",
            rustflags: service_rustflags,
            args: &[
                "--features",
                "sunlightos",
                "--no-default-features",
                "--release",
            ],
        },
        EmbeddedBinary {
            package: "sunlight-kvctl",
            output: "sunlight-kvctl",
            rustflags: service_rustflags,
            args: &[
                "--features",
                "sunlightos",
                "--no-default-features",
                "--release",
            ],
        },
        EmbeddedBinary {
            package: "sunlight-tls",
            output: "sunlight-tls",
            rustflags: tls_rustflags,
            args: &[
                "--features",
                "sunlightos",
                "--no-default-features",
                "--release",
            ],
        },
        EmbeddedBinary {
            package: "certificatectl",
            output: "certificatectl",
            rustflags: service_rustflags,
            args: &[
                "--features",
                "sunlightos",
                "--no-default-features",
                "--release",
            ],
        },
        EmbeddedBinary {
            package: "sunlight-uac",
            output: "uac_service",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-sm",
            output: "sunlight-sm",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-solar",
            output: "solar",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-sunsay",
            output: "sunlight-sunsay",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-zoxide",
            output: "z",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-dict",
            output: "dict",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-hangman",
            output: "hangman",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-runner",
            output: "sunlight-runner",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sun-exec",
            output: "sun-exec",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sun-open",
            output: "sun-open",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-terminal",
            output: "sunlight-terminal",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-chronos",
            output: "sunlight-chronos",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-tasks",
            output: "sunlight-tasks",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-bench",
            output: "sunbench",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-calculator",
            output: "calculator",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-widget-gallery",
            output: "widget-gallery",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-calendar",
            output: "sunlight-calendar",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-reminders",
            output: "sunlight-reminders",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-devices",
            output: "sunlight-devices",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "rappid-rabbit",
            output: "rappid-rabbit",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-api-lab",
            output: "sunlight-api-lab",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-light-lens",
            output: "light-lens",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-vortex-shell",
            output: "sunlight-vortex-shell",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "mezzo",
            output: "mezzo",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-control-panel",
            output: "control-panel",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "sunlight-thumbd",
            output: "sunlight-thumbd",
            rustflags: service_rustflags,
            args: &["--release"],
        },
        EmbeddedBinary {
            package: "cpu-utils",
            output: "cpufeat",
            rustflags: service_rustflags,
            args: &["--release"],
        },
    ];

    for bin in binaries {
        let path = release_dir.join(bin.output);
        let needs_build = !path.exists() || !userspace_elf_looks_valid(&path);
        if needs_build {
            if path.exists() {
                println!(
                    "cargo:warning=rebuilding embedded binary {} (missing or kernel-linked ELF)",
                    bin.package
                );
            }
            build_package(
                workspace_root,
                scratch_target_dir.as_path(),
                release_dir.as_path(),
                bin,
            );
        }
    }

    println!("cargo:rerun-if-changed=build.rs");
}

/// Reject ELFs linked with the kernel linker script (vaddr in HHDM / -2GiB).
/// Manual `cargo build -p <svc>` without userspace RUSTFLAGS produces those
/// and previously stuck forever because build.rs only checked `exists()`.
fn userspace_elf_looks_valid(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    if bytes.len() < 64 || &bytes[0..4] != b"\x7fELF" {
        return false;
    }
    // ELF64 e_entry at offset 24, little-endian.
    if bytes.len() < 32 {
        return false;
    }
    let entry = u64::from_le_bytes(bytes[24..32].try_into().unwrap_or([0; 8]));
    // Userspace services link at 0x400000; kernel image lives in high canonical space.
    entry >= 0x400000 && entry < 0x0000_8000_0000_0000
}

fn build_package(
    workspace_root: &Path,
    scratch_target_dir: &Path,
    release_dir: &Path,
    bin: EmbeddedBinary<'_>,
) {
    println!(
        "cargo:warning=prebuilding embedded binary {} for kernel",
        bin.package
    );

    let scratch_release_dir = scratch_target_dir
        .join("x86_64-unknown-none")
        .join("release");
    fs::create_dir_all(&scratch_release_dir).expect("failed to create scratch target dir");

    let mut cmd = Command::new("cargo");
    cmd.current_dir(workspace_root)
        .env("CARGO_TARGET_DIR", scratch_target_dir)
        // The parent Cargo process exports its target-specific encoded flags
        // to build scripts. Do not let those kernel linker flags override the
        // explicit user-space flags below in the child Cargo invocation.
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env("RUSTFLAGS", bin.rustflags)
        .arg("build")
        .arg("--package")
        .arg(bin.package)
        .args(bin.args);

    let status = cmd.status().unwrap_or_else(|err| {
        panic!("failed to invoke cargo for {}: {err}", bin.package);
    });

    if !status.success() {
        panic!("failed to build embedded binary {}", bin.package);
    }

    let built_path = scratch_release_dir.join(bin.output);
    let dest_path = release_dir.join(bin.output);
    fs::create_dir_all(release_dir).expect("failed to create embedded output dir");
    fs::copy(&built_path, &dest_path).unwrap_or_else(|err| {
        panic!(
            "failed to copy embedded binary {} from {} to {}: {err}",
            bin.package,
            built_path.display(),
            dest_path.display()
        );
    });
}
