//! PostScript/EPS conversion followed by MuPDF rendering.
use super::mupdf_backend::MuPdfBackend;
use crate::backend::{DocumentBackend, DocumentHandle};
use anyhow::{ensure, Context, Result};
#[cfg(not(target_env = "musl"))]
use std::process::{Command, Stdio};
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

/// Uses the `gs` executable to convert PS/EPS to a temporary PDF.
pub struct GhostscriptBackend;

struct TemporaryPdf(PathBuf);
impl Drop for TemporaryPdf {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl DocumentBackend for GhostscriptBackend {
    fn open(&self, path: &Path) -> Result<Box<dyn DocumentHandle>> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let source = std::fs::canonicalize(path).context("locate PostScript source")?;
        let output = std::env::temp_dir().join(format!(
            "sunlight-docserv-{}-{}.pdf",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .context("reserve conversion output")?;
        let output = TemporaryPdf(output);
        convert(&source, &output.0)?;
        MuPdfBackend
            .open(&output.0)
            .context("open converted PostScript PDF")
    }

    fn supported_extensions(&self) -> &[&str] {
        &["ps", "eps"]
    }
}

#[cfg(not(target_env = "musl"))]
fn convert(source: &Path, output: &Path) -> Result<()> {
    let mut child = Command::new("gs")
        .args([
            "-q",
            "-dSAFER",
            "-dBATCH",
            "-dNOPAUSE",
            "-dEPSCrop",
            "-sDEVICE=pdfwrite",
        ])
        .arg(format!("-sOutputFile={}", output.display()))
        .arg("--")
        .arg(source)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .context("start Ghostscript (install gs for PS/EPS support)")?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().context("wait for Ghostscript")? {
            break status;
        }
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Ghostscript conversion timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    ensure!(status.success(), "Ghostscript conversion failed: {status}");
    Ok(())
}

#[cfg(target_env = "musl")]
fn convert(source: &Path, output: &Path) -> Result<()> {
    use std::ffi::CString;
    let path = CString::new("/bin/gs")?;
    let arguments: Vec<CString> = [
        "gs",
        "-q",
        "-dSAFER",
        "-dBATCH",
        "-dNOPAUSE",
        "-dEPSCrop",
        "-sDEVICE=pdfwrite",
    ]
    .iter()
    .map(|s| CString::new(*s))
    .collect::<std::result::Result<_, _>>()?;
    let mut arguments = arguments;
    arguments.push(CString::new(format!("-sOutputFile={}", output.display()))?);
    arguments.push(CString::new("--")?);
    arguments.push(CString::new(
        source
            .to_str()
            .context("PostScript source path must be UTF-8")?,
    )?);
    let mut pointers: Vec<*const std::ffi::c_char> = arguments.iter().map(|s| s.as_ptr()).collect();
    pointers.push(core::ptr::null());
    let child =
        unsafe { native_call(39, path.as_ptr() as u64, pointers.as_ptr() as u64, u64::MAX) };
    ensure!(child < u64::MAX - 4096, "native Ghostscript spawn failed");
    let start = Instant::now();
    while unsafe { native_call(25, child, 0, 0) } == 1 {
        if start.elapsed() > Duration::from_secs(30) {
            unsafe {
                native_call(72, child, 9, 0);
            }
            anyhow::bail!("Ghostscript conversion timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let code = unsafe { native_call(32, child, 0, 0) };
    ensure!(
        code == 0,
        "Ghostscript conversion failed with exit code {code}"
    );
    Ok(())
}

#[cfg(target_env = "musl")]
unsafe fn native_call(number: u64, a: u64, b: u64, c: u64) -> u64 {
    let result;
    // Native register ABI; CString storage lives across the spawn call. These
    // operations do not return an IPC message or modify additional registers.
    core::arch::asm!("syscall",
        inlateout("rax") number | sunlight_ipc::NATIVE_SYSCALL_TAG => result,
        inlateout("rdi") a => _, inlateout("rsi") b => _, inlateout("rdx") c => _,
        lateout("rcx") _, lateout("r11") _);
    result
}
