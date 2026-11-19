#!/usr/bin/env python3
"""Run the production header formatter against deterministic telemetry and time."""
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
source = (root / 'services/tty_server/src/main.rs').read_text()
header = source[source.index('static mut HEADER:'):source.index('fn utc_header_clock(')]
helpers = source[source.index('fn copy_into('):]
# Only the two numeric/text formatting helpers, before the next item.
helpers = helpers[:helpers.index('\n}', helpers.index('fn fmt_u64(')) + 2]
stubs = r'''
static mut NOW: u64 = 10000;
fn monotonic_millis() -> u64 { unsafe { NOW } }
fn get_time_utc() -> u64 { 0 }
unsafe fn try_local_clock(dst: &mut [u8], _: u64) -> (usize, bool) {
    (copy_into(dst, b"12:34 UTC"), false)
}
struct Snapshot { sample_time_ns: u64, cpu_capacity_ns: u64,
    cpu_used_bp: u16, used_ram_kb: u64, total_ram_kb: u64 }
struct Telemetry(Snapshot);
impl Telemetry {
    fn poll(&mut self) -> bool { false }
    fn snapshot(&self) -> &Snapshot { &self.0 }
}
mod sunlight_networkd {
    #[derive(Clone, Copy)]
    pub enum DerivedConnectionState { Connected, Configuring, CableDisconnected, Unavailable, Error }
    #[derive(Clone, Copy)]
    pub struct NetworkPanelSummary { pub captured_monotonic_time: u64, pub state: DerivedConnectionState }
    pub static mut STATE: Option<DerivedConnectionState> = None;
    pub struct NetworkClient;
    impl NetworkClient {
        pub fn new() -> Self { Self }
        pub fn panel_summary(&self) -> Result<NetworkPanelSummary, ()> {
            unsafe { STATE.map(|state| NetworkPanelSummary {
                captured_monotonic_time: super::monotonic_millis(), state }).ok_or(()) }
        }
    }
}
fn main() { unsafe {
    let mut telemetry = Some(Telemetry(Snapshot { sample_time_ns: 10_000_000_000,
        cpu_capacity_ns: 100, cpu_used_bp: 3750, used_ram_kb: 42, total_ram_kb: 100 }));
    assert!(refresh_header(&mut telemetry));
    assert_eq!(&HEADER.0[..HEADER.1], b"CPU 37% | RAM 42% | 12:34 UTC | NET --");
    assert!(!refresh_header(&mut telemetry));
    NOW += 1000;
    telemetry.as_mut().unwrap().0.cpu_capacity_ns = 0;
    assert!(refresh_header(&mut telemetry));
    assert!(HEADER.0[..HEADER.1].starts_with(b"CPU -- | RAM 42%"));
    NOW += 3001;
    assert!(refresh_header(&mut telemetry));
    assert!(HEADER.0[..HEADER.1].starts_with(b"CPU -- | RAM --"));
    NOW += 1000;
    assert!(refresh_header(&mut None));
    assert!(HEADER.0[..HEADER.1].starts_with(b"CPU -- | RAM --"));
    use sunlight_networkd::DerivedConnectionState::*;
    for (state, label) in [(Connected, "up"), (Configuring, "config"),
            (CableDisconnected, "down"), (Unavailable, "none"), (Error, "--")] {
        NOW = (NOW / 2000 + 1) * 2000;
        sunlight_networkd::STATE = Some(state);
        assert!(refresh_header(&mut None));
        assert!(std::str::from_utf8(&HEADER.0[..HEADER.1]).unwrap().ends_with(label));
    }
    NOW += 4000;
    sunlight_networkd::STATE = None;
    assert!(refresh_header(&mut None));
    assert!(std::str::from_utf8(&HEADER.0[..HEADER.1]).unwrap().ends_with("NET --"));
    println!("PASS: production header cadence, CPU baseline, stale/unavailable data, network states and loss");
}}
'''
with tempfile.TemporaryDirectory(prefix='tty-status-') as directory:
    path = Path(directory)
    (path / 'main.rs').write_text(stubs + header + helpers)
    subprocess.run(['rustc', '--edition=2021', '-Awarnings', str(path / 'main.rs'), '-o', str(path / 'test')], check=True)
    subprocess.run([str(path / 'test')], check=True)
