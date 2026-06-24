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
            package: "sunlight-deviced",
            output: "devicectl",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "devicectl"],
        },
        EmbeddedBinary {
            package: "sunlight-networkd",
            output: "networkctl",
            rustflags: service_rustflags,
            args: &["--release", "--bin", "networkctl"],
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
    ];

    for bin in binaries {
        let path = release_dir.join(bin.output);
        if !path.exists() {
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
