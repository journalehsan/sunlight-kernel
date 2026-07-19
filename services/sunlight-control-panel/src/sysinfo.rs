//! Structured system-information snapshot for Control Panel About pages.
//!
//! Collects the same underlying data used by `sysinfo`, `free`/`freezram`,
//! `sysfetch`, and `uname` without parsing their human-readable output.

use core::fmt::Write;
use sunlight_ipc::{
    map_telemetry, query_display_metrics, swap_aggregate_diagnostics, CapabilityToken,
    DisplayMetrics, ScreenBackend,
};
use sunlight_libc as libc;

const NA: &str = "Not available";
const OS_NAME: &str = "SunlightOS";
const KERNEL_NAME: &str = "SunlightX";
const OS_DESCRIPTION: &str = "A modern Rust microkernel operating system";
const OS_RELEASE_STAGE: &str = "Alpha";

/// Desktop / core component entry for data-driven About OS UI.
#[derive(Clone, Copy)]
pub struct ComponentEntry {
    pub name: &'static str,
    /// `None` when no reliable version metadata is available.
    pub version: Option<&'static str>,
}

/// Components known to exist in this repository, with shared workspace build
/// version when that is the only reliable identifier.
pub const CORE_COMPONENTS: &[ComponentEntry] = &[
    ComponentEntry {
        name: "Vortex Shell",
        version: Some(env!("CARGO_PKG_VERSION")),
    },
    ComponentEntry {
        name: "Helios",
        version: Some(env!("CARGO_PKG_VERSION")),
    },
    ComponentEntry {
        name: "Display Service",
        version: Some(env!("CARGO_PKG_VERSION")),
    },
    ComponentEntry {
        name: "Base System",
        version: Some(env!("CARGO_PKG_VERSION")),
    },
];

#[derive(Clone, Copy)]
pub struct FixedStr<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> FixedStr<N> {
    pub const fn empty() -> Self {
        Self {
            buf: [0u8; N],
            len: 0,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    pub fn push_str(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let n = bytes.len().min(N.saturating_sub(self.len));
        self.buf[self.len..self.len + n].copy_from_slice(&bytes[..n]);
        self.len += n;
    }

    pub fn set(&mut self, s: &str) {
        self.clear();
        self.push_str(s);
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> Write for FixedStr<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.push_str(s);
        Ok(())
    }
}

/// Structured snapshot used by both About pages and copy-to-clipboard reports.
pub struct SystemInfoSnapshot {
    // Host / computer
    pub hostname: FixedStr<64>,
    pub platform: FixedStr<32>,
    pub architecture: FixedStr<16>,
    pub cpu_model: FixedStr<64>,
    pub cpu_cores: Option<u32>,

    // Memory (KB)
    pub mem_total_kb: Option<u64>,
    pub mem_used_kb: Option<u64>,
    pub mem_available_kb: Option<u64>,

    // ZRAM / swap (KB)
    pub zram_enabled: Option<bool>,
    pub zram_capacity_kb: Option<u64>,
    pub zram_used_kb: Option<u64>,
    pub zram_compressed_kb: Option<u64>,

    // Graphics / display
    pub graphics_adapter: FixedStr<48>,
    pub graphics_backend: FixedStr<32>,
    pub display_w: Option<u32>,
    pub display_h: Option<u32>,

    // Runtime
    pub uptime_secs: Option<u64>,

    // OS / kernel identity
    pub os_name: FixedStr<32>,
    pub os_version: FixedStr<32>,
    pub os_build: FixedStr<64>,
    pub os_edition: FixedStr<32>,
    pub os_channel: FixedStr<32>,
    pub os_description: FixedStr<96>,
    pub kernel_name: FixedStr<32>,
    pub kernel_release: FixedStr<32>,
    pub kernel_version: FixedStr<64>,
    pub kernel_arch: FixedStr<16>,
    pub uname_string: FixedStr<192>,
}

impl SystemInfoSnapshot {
    pub fn collect(display_ep: Option<CapabilityToken>, screen_w: u32, screen_h: u32) -> Self {
        let mut s = Self::blank();

        s.os_name.set(OS_NAME);
        s.kernel_name.set(KERNEL_NAME);
        s.os_version.set(env!("CARGO_PKG_VERSION"));
        s.kernel_release.set(env!("CARGO_PKG_VERSION"));
        s.os_edition.set(OS_RELEASE_STAGE);
        s.os_channel.set(OS_RELEASE_STAGE);
        s.os_description.set(OS_DESCRIPTION);

        if let Some(ident) = option_env!("COOKBOOK_SOURCE_IDENT") {
            if !ident.is_empty() {
                s.os_build.set(ident);
                s.kernel_version.set(ident);
            }
        }
        if s.kernel_version.is_empty() {
            s.kernel_version.set("SunlightX build");
        }

        let arch = machine_arch();
        s.architecture.set(arch);
        s.kernel_arch.set(arch);

        read_hostname(&mut s.hostname);
        if s.hostname.is_empty() {
            s.hostname.set("sunlight");
        }

        read_cpu_brand(&mut s.cpu_model);
        s.platform.set(detect_platform());

        // Memory / uptime via SYS_SYSINFO (same source as `free` and sysfetch).
        match libc::sysinfo() {
            Ok(info) => {
                let total = info.total_ram_kb;
                let used = info.used_ram_kb.min(total);
                let free = total.saturating_sub(used);
                s.mem_total_kb = Some(total);
                s.mem_used_kb = Some(used);
                s.mem_available_kb = Some(free);
                s.uptime_secs = Some(info.uptime_secs);

                // Swap totals from sysinfo (ZRAM-backed when swapd is up).
                s.zram_capacity_kb = Some(info.swap_total_kb);
                s.zram_used_kb = Some(info.swap_used_kb.min(info.swap_total_kb));
                if info.swap_used_kb > 0 {
                    s.zram_compressed_kb = Some(info.swap_compressed_kb);
                }
                s.zram_enabled = Some(info.swap_total_kb > 0);
            }
            Err(_) => {}
        }

        // Richer ZRAM capacity/usage when swap diagnostics are available.
        if let Some(diag) = swap_aggregate_diagnostics() {
            if diag.service_configured != 0 || diag.active_pool_count != 0 {
                s.zram_enabled = Some(diag.active_pool_count != 0 || diag.service_configured != 0);
                let cap_kb = diag.configured_logical_pages.saturating_mul(4);
                let used_kb = diag.stored_pages.saturating_mul(4);
                let comp_kb = diag.compressed_bytes.saturating_add(1023) / 1024;
                if cap_kb > 0 {
                    s.zram_capacity_kb = Some(cap_kb);
                }
                s.zram_used_kb = Some(used_kb);
                if comp_kb > 0 {
                    s.zram_compressed_kb = Some(comp_kb);
                }
            }
            if diag.detected_online_cpus > 0 {
                s.cpu_cores = Some(diag.detected_online_cpus as u32);
            }
        }

        // CPU count from telemetry page when available.
        if s.cpu_cores.is_none() {
            if let Some(count) = telemetry_cpu_count() {
                s.cpu_cores = Some(count);
            }
        }

        // Display metrics (resolution + backend label).
        let metrics = display_ep
            .and_then(query_display_metrics)
            .unwrap_or(DisplayMetrics {
                width_px: screen_w,
                height_px: screen_h,
                ..DisplayMetrics::safe_fallback()
            });
        s.display_w = Some(metrics.width_px.max(1));
        s.display_h = Some(metrics.height_px.max(1));
        s.graphics_backend.set(backend_label(metrics.backend));
        // Adapter name is not exposed as a stable string today; platform is
        // a reliable hint only for common VM graphics stacks.
        match s.platform.as_str() {
            "VMware" => {
                if metrics.backend == ScreenBackend::VmwareSvga {
                    s.graphics_adapter.set("VMware SVGA II");
                } else {
                    s.graphics_adapter.set("VMware virtual display");
                }
            }
            "QEMU" => {
                if metrics.backend == ScreenBackend::VirtioGpu {
                    s.graphics_adapter.set("VirtIO GPU");
                } else {
                    s.graphics_adapter.set("QEMU virtual display");
                }
            }
            "Physical computer" => {
                if metrics.backend == ScreenBackend::VirtioGpu {
                    s.graphics_adapter.set("VirtIO GPU");
                }
                // else leave empty → Not available
            }
            _ => {}
        }

        build_uname_string(&mut s);
        s
    }

    fn blank() -> Self {
        Self {
            hostname: FixedStr::empty(),
            platform: FixedStr::empty(),
            architecture: FixedStr::empty(),
            cpu_model: FixedStr::empty(),
            cpu_cores: None,
            mem_total_kb: None,
            mem_used_kb: None,
            mem_available_kb: None,
            zram_enabled: None,
            zram_capacity_kb: None,
            zram_used_kb: None,
            zram_compressed_kb: None,
            graphics_adapter: FixedStr::empty(),
            graphics_backend: FixedStr::empty(),
            display_w: None,
            display_h: None,
            uptime_secs: None,
            os_name: FixedStr::empty(),
            os_version: FixedStr::empty(),
            os_build: FixedStr::empty(),
            os_edition: FixedStr::empty(),
            os_channel: FixedStr::empty(),
            os_description: FixedStr::empty(),
            kernel_name: FixedStr::empty(),
            kernel_release: FixedStr::empty(),
            kernel_version: FixedStr::empty(),
            kernel_arch: FixedStr::empty(),
            uname_string: FixedStr::empty(),
        }
    }

    pub fn field_or_na<'a>(value: &'a str) -> &'a str {
        if value.is_empty() {
            NA
        } else {
            value
        }
    }

    pub fn mem_usage_ratio(&self) -> f32 {
        match (self.mem_used_kb, self.mem_total_kb) {
            (Some(used), Some(total)) if total > 0 => {
                (used as f32 / total as f32).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }

    pub fn format_kb_human(kb: u64, out: &mut FixedStr<32>) {
        out.clear();
        if kb >= 1024 * 1024 {
            let gib_x10 = (kb * 10) / (1024 * 1024);
            let _ = write!(out, "{}.{} GiB", gib_x10 / 10, gib_x10 % 10);
        } else if kb >= 1024 {
            let mib_x10 = (kb * 10) / 1024;
            let _ = write!(out, "{}.{} MiB", mib_x10 / 10, mib_x10 % 10);
        } else {
            let _ = write!(out, "{} KiB", kb);
        }
    }

    pub fn format_uptime(secs: u64, out: &mut FixedStr<48>) {
        out.clear();
        let h = secs / 3600;
        let m = (secs / 60) % 60;
        let s = secs % 60;
        if h > 0 {
            let _ = write!(out, "{}h {}m", h, m);
        } else if m > 0 {
            let _ = write!(out, "{}m {}s", m, s);
        } else {
            let _ = write!(out, "{}s", s);
        }
    }

    /// Plain-text summary for the clipboard (About This Computer).
    pub fn copy_computer_summary(&self, out: &mut FixedStr<1024>) {
        out.clear();
        let _ = writeln!(out, "About This Computer");
        let _ = writeln!(out, "Computer: {}", Self::field_or_na(self.hostname.as_str()));
        let _ = writeln!(out, "Platform: {}", Self::field_or_na(self.platform.as_str()));
        let _ = writeln!(
            out,
            "Architecture: {}",
            Self::field_or_na(self.architecture.as_str())
        );
        let _ = writeln!(out, "Processor: {}", Self::field_or_na(self.cpu_model.as_str()));
        match self.cpu_cores {
            Some(n) => {
                let _ = writeln!(out, "CPU cores: {}", n);
            }
            None => {
                let _ = writeln!(out, "CPU cores: {}", NA);
            }
        }
        self.write_mem_lines(out);
        self.write_zram_lines(out);
        let _ = writeln!(
            out,
            "Graphics: {}",
            Self::field_or_na(self.graphics_adapter.as_str())
        );
        let _ = writeln!(
            out,
            "Display backend: {}",
            Self::field_or_na(self.graphics_backend.as_str())
        );
        match (self.display_w, self.display_h) {
            (Some(w), Some(h)) => {
                let _ = writeln!(out, "Resolution: {} x {}", w, h);
            }
            _ => {
                let _ = writeln!(out, "Resolution: {}", NA);
            }
        }
        match self.uptime_secs {
            Some(u) => {
                let mut ub = FixedStr::<48>::empty();
                Self::format_uptime(u, &mut ub);
                let _ = writeln!(out, "Uptime: {}", ub.as_str());
            }
            None => {
                let _ = writeln!(out, "Uptime: {}", NA);
            }
        }
        let _ = writeln!(
            out,
            "Kernel: {} {}",
            Self::field_or_na(self.kernel_name.as_str()),
            Self::field_or_na(self.kernel_release.as_str())
        );
    }

    /// Plain-text system report for the clipboard (About SunlightOS).
    pub fn copy_system_report(&self, out: &mut FixedStr<1536>) {
        out.clear();
        let _ = writeln!(out, "About SunlightOS — System Report");
        let _ = writeln!(out, "OS: {}", Self::field_or_na(self.os_name.as_str()));
        let _ = writeln!(out, "Version: {}", Self::field_or_na(self.os_version.as_str()));
        let _ = writeln!(out, "Build: {}", Self::field_or_na(self.os_build.as_str()));
        let _ = writeln!(out, "Edition: {}", Self::field_or_na(self.os_edition.as_str()));
        let _ = writeln!(out, "Channel: {}", Self::field_or_na(self.os_channel.as_str()));
        let _ = writeln!(
            out,
            "Architecture: {}",
            Self::field_or_na(self.architecture.as_str())
        );
        let _ = writeln!(
            out,
            "Kernel name: {}",
            Self::field_or_na(self.kernel_name.as_str())
        );
        let _ = writeln!(
            out,
            "Kernel release: {}",
            Self::field_or_na(self.kernel_release.as_str())
        );
        let _ = writeln!(
            out,
            "Kernel build: {}",
            Self::field_or_na(self.kernel_version.as_str())
        );
        let _ = writeln!(
            out,
            "Kernel arch: {}",
            Self::field_or_na(self.kernel_arch.as_str())
        );
        match self.cpu_cores {
            Some(n) => {
                let _ = writeln!(out, "Logical CPUs: {}", n);
            }
            None => {
                let _ = writeln!(out, "Logical CPUs: {}", NA);
            }
        }
        let _ = writeln!(out, "Components:");
        for c in CORE_COMPONENTS {
            match c.version {
                Some(v) => {
                    let _ = writeln!(out, "  - {}: {}", c.name, v);
                }
                None => {
                    let _ = writeln!(out, "  - {}: {}", c.name, NA);
                }
            }
        }
        let _ = writeln!(
            out,
            "Technical kernel string: {}",
            Self::field_or_na(self.uname_string.as_str())
        );
        let _ = writeln!(out, "");
        let _ = writeln!(out, "--- Host snapshot ---");
        let _ = writeln!(out, "Computer: {}", Self::field_or_na(self.hostname.as_str()));
        let _ = writeln!(out, "Platform: {}", Self::field_or_na(self.platform.as_str()));
        let _ = writeln!(out, "Processor: {}", Self::field_or_na(self.cpu_model.as_str()));
        self.write_mem_lines(out);
        self.write_zram_lines(out);
        match self.uptime_secs {
            Some(u) => {
                let mut ub = FixedStr::<48>::empty();
                Self::format_uptime(u, &mut ub);
                let _ = writeln!(out, "Uptime: {}", ub.as_str());
            }
            None => {
                let _ = writeln!(out, "Uptime: {}", NA);
            }
        }
    }

    fn write_mem_lines<const N: usize>(&self, out: &mut FixedStr<N>) {
        let mut buf = FixedStr::<32>::empty();
        match self.mem_total_kb {
            Some(v) => {
                Self::format_kb_human(v, &mut buf);
                let _ = writeln!(out, "Memory total: {}", buf.as_str());
            }
            None => {
                let _ = writeln!(out, "Memory total: {}", NA);
            }
        }
        match self.mem_used_kb {
            Some(v) => {
                Self::format_kb_human(v, &mut buf);
                let _ = writeln!(out, "Memory used: {}", buf.as_str());
            }
            None => {
                let _ = writeln!(out, "Memory used: {}", NA);
            }
        }
        match self.mem_available_kb {
            Some(v) => {
                Self::format_kb_human(v, &mut buf);
                let _ = writeln!(out, "Memory available: {}", buf.as_str());
            }
            None => {
                let _ = writeln!(out, "Memory available: {}", NA);
            }
        }
    }

    fn write_zram_lines<const N: usize>(&self, out: &mut FixedStr<N>) {
        match self.zram_enabled {
            Some(true) => {
                let _ = writeln!(out, "ZRAM: enabled");
            }
            Some(false) => {
                let _ = writeln!(out, "ZRAM: disabled");
            }
            None => {
                let _ = writeln!(out, "ZRAM: {}", NA);
            }
        }
        let mut buf = FixedStr::<32>::empty();
        match self.zram_capacity_kb {
            Some(v) => {
                Self::format_kb_human(v, &mut buf);
                let _ = writeln!(out, "ZRAM capacity: {}", buf.as_str());
            }
            None => {
                let _ = writeln!(out, "ZRAM capacity: {}", NA);
            }
        }
        match self.zram_used_kb {
            Some(v) => {
                Self::format_kb_human(v, &mut buf);
                let _ = writeln!(out, "ZRAM used: {}", buf.as_str());
            }
            None => {
                let _ = writeln!(out, "ZRAM used: {}", NA);
            }
        }
        if let Some(v) = self.zram_compressed_kb {
            if v > 0 {
                Self::format_kb_human(v, &mut buf);
                let _ = writeln!(out, "ZRAM compressed: {}", buf.as_str());
            }
        }
    }
}

fn machine_arch() -> &'static str {
    option_env!("TARGET")
        .and_then(|t| t.split('-').next())
        .unwrap_or("x86_64")
}

fn backend_label(backend: ScreenBackend) -> &'static str {
    match backend {
        ScreenBackend::VirtioGpu => "virtio-gpu",
        ScreenBackend::LimineFramebuffer => "limine-framebuffer",
        ScreenBackend::Fallback => "fallback",
        ScreenBackend::VmwareSvga => "vmware-svga",
    }
}

fn read_hostname(out: &mut FixedStr<64>) {
    out.clear();
    let fd = match libc::open(b"/etc/hostname") {
        Ok(fd) => fd,
        Err(_) => return,
    };
    let mut buf = [0u8; 128];
    let n = match libc::read(fd, &mut buf) {
        Ok(n) => n,
        Err(_) => {
            let _ = libc::close(fd);
            return;
        }
    };
    let _ = libc::close(fd);
    let mut end = 0usize;
    while end < n {
        let b = buf[end];
        if b == b'\n' || b == b'\r' || b == 0 {
            break;
        }
        end += 1;
    }
    if end > 0 {
        if let Ok(s) = core::str::from_utf8(&buf[..end]) {
            out.set(s);
        }
    }
}

fn read_cpu_brand(out: &mut FixedStr<64>) {
    out.clear();
    let mut raw = [0u8; 48];
    for (i, leaf) in (0x8000_0002u32..=0x8000_0004).enumerate() {
        let r = core::arch::x86_64::__cpuid(leaf);
        for (j, reg) in [r.eax, r.ebx, r.ecx, r.edx].into_iter().enumerate() {
            raw[i * 16 + j * 4..i * 16 + j * 4 + 4].copy_from_slice(&reg.to_le_bytes());
        }
    }
    let start = raw.iter().position(|&b| b != b' ' && b != 0).unwrap_or(0);
    let end = raw.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
    if start < end {
        if let Ok(s) = core::str::from_utf8(&raw[start..end]) {
            out.set(s.trim());
        }
    }
}

/// Hypervisor / platform label via CPUID (same approach as sunlight-bench).
fn detect_platform() -> &'static str {
    let r1 = core::arch::x86_64::__cpuid(1);
    if r1.ecx & (1 << 31) == 0 {
        return "Physical computer";
    }
    let hv = core::arch::x86_64::__cpuid(0x4000_0000);
    let ebx = hv.ebx;
    let ecx = hv.ecx;
    let edx = hv.edx;
    // "VMwareVMware"
    if ebx == 0x6177_4D56 && ecx == 0x4D56_6572 && edx == 0x6572_6177 {
        return "VMware";
    }
    // "KVMKVMKVM\0\0\0"
    if ebx == 0x4B4D_564B && ecx == 0x564B_4D56 && edx == 0x0000_004D {
        return "QEMU";
    }
    // "TCGTCGTCGTCG"
    if ebx == 0x5447_4354 && ecx == 0x5447_4354 && edx == 0x5447_4354 {
        return "QEMU";
    }
    "Virtual machine"
}

fn telemetry_cpu_count() -> Option<u32> {
    let ptr = map_telemetry();
    if ptr.is_null() {
        return None;
    }
    // Layout matches kernel TelemetryPage / sunlight-telemetry:
    // magic u64, version u32, sequence u32, uptime u64, total_ram u64,
    // used_ram u64, zram_orig u64, zram_comp u64, net_rx u64, net_tx u64,
    // tick_hz u32, cpu_count u8, ...
    const MAGIC: u64 = 0x5355_4E4C_5449_4D45;
    unsafe {
        let magic = core::ptr::read_unaligned(ptr as *const u64);
        if magic != MAGIC {
            return None;
        }
        // Offset of cpu_count: 8+4+4+8*7+4 = 8+8+56+4 = 76
        let cpu_count = *ptr.add(76);
        if cpu_count == 0 {
            None
        } else {
            Some(cpu_count as u32)
        }
    }
}

fn build_uname_string(s: &mut SystemInfoSnapshot) {
    s.uname_string.clear();
    let _ = write!(
        s.uname_string,
        "{} {} {} {} {} {}",
        s.kernel_name.as_str(),
        s.hostname.as_str(),
        s.kernel_release.as_str(),
        s.kernel_version.as_str(),
        s.architecture.as_str(),
        s.os_name.as_str()
    );
}
