//! Intel HD Audio playback driver (userspace).
//!
//! The kernel remains responsible for PCI configuration and for granting
//! physically contiguous DMA + uncached MMIO. This module programs the
//! controller, talks to one codec, and owns the BDL / PCM ring.

use crate::{
    pcm::{apply_gain_s16le, generate_sine_s16le_stereo, FRAME_BYTES, NATIVE_RATE_HZ},
    AudioCapabilities, AudioDeviceState, AudioError, OutputDeviceKind,
};
use sunlight_ipc::{dma_alloc, hda_info, map_mmio, monotonic_millis, process_yield};

const PAGE: usize = 4096;
const PERIODS: usize = 4;
const PERIOD_BYTES: usize = PAGE;
const RING_BYTES: usize = PERIODS * PERIOD_BYTES;
const DMA_PAGES: usize = 1 + PERIODS; // page 0 = CORB/RIRB/BDL, pages 1.. = PCM
const BDL_ALIGN: usize = 128;
const CORB_ENTRIES: usize = 256;
const RIRB_ENTRIES: usize = 256;

const GCTL: usize = 0x08;
const STATESTS: usize = 0x0e;
const INTCTL: usize = 0x20;
const INTSTS: usize = 0x24;
const CORBLBASE: usize = 0x40;
const CORBUBASE: usize = 0x44;
const CORBWP: usize = 0x48;
const CORBRP: usize = 0x4a;
const CORBCTL: usize = 0x4c;
const CORBSIZE: usize = 0x4e;
const RIRBLBASE: usize = 0x50;
const RIRBUBASE: usize = 0x54;
const RIRBWP: usize = 0x58;
const RINTCNT: usize = 0x5a;
const RIRBCTL: usize = 0x5c;
const RIRBSTS: usize = 0x5d;
const RIRBSIZE: usize = 0x5e;
const ICOI: usize = 0x60;
const ICII: usize = 0x64;
const ICIS: usize = 0x68;

const GCTL_CRST: u32 = 1;
const ICIS_ICB: u16 = 1;
const ICIS_IRV: u16 = 2;

const VERB_GET_PARAM: u32 = 0xf00;
const VERB_SET_STREAM_FORMAT: u32 = 0x2;
const VERB_SET_AMP_GAIN: u32 = 0x3;
const VERB_SET_POWER: u32 = 0x705;
const VERB_SET_CHAN_STREAMID: u32 = 0x706;
const VERB_SET_PIN_WIDGET: u32 = 0x707;
const VERB_SET_EAPD: u32 = 0x70c;

const PARAM_VENDOR: u32 = 0x00;
const PARAM_SUBORD: u32 = 0x04;
const PARAM_FNGROUP: u32 = 0x05;
const PARAM_WIDGET_CAP: u32 = 0x09;
const PARAM_PIN_CAP: u32 = 0x0c;
const PARAM_CONN_LIST_LEN: u32 = 0x0e;
const PARAM_AMP_OUT_CAP: u32 = 0x12;

const WIDGET_AUDIO_OUT: u32 = 0x0;
const WIDGET_PIN: u32 = 0x4;
const STREAM_ID: u8 = 1;
const HDA_FMT_48K_S16_STEREO: u16 = 0x0011;
const MAX_WIDGETS: usize = 32;
const CMD_TIMEOUT_MS: u64 = 50;
const RESET_TIMEOUT_MS: u64 = 200;
const CODEC_DETECT_TIMEOUT_MS: u64 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HdaError {
    NoDevice,
    MapFailed,
    DmaFailed,
    ResetTimeout,
    CodecTimeout,
    NoCodec,
    NoOutputPath,
    BadResource,
}

impl HdaError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoDevice => "no-device",
            Self::MapFailed => "mmio-map-failed",
            Self::DmaFailed => "dma-allocation-failed",
            Self::ResetTimeout => "controller-reset-timeout",
            Self::CodecTimeout => "codec-command-timeout",
            Self::NoCodec => "codec-not-detected",
            Self::NoOutputPath => "output-path-not-found",
            Self::BadResource => "invalid-device-resource",
        }
    }
}

impl From<HdaError> for AudioError {
    fn from(err: HdaError) -> Self {
        match err {
            HdaError::NoDevice => AudioError::DeviceUnavailable,
            HdaError::ResetTimeout | HdaError::CodecTimeout => AudioError::Timeout,
            _ => AudioError::DeviceFailed,
        }
    }
}

#[repr(C, packed)]
struct BdlEntry {
    addr: u64,
    length: u32,
    flags: u32,
}

pub struct HdaPlayback {
    mmio: *mut u8,
    dma: *mut u8,
    dma_phys: u64,
    bar_phys: u64,
    bar_size: u64,
    vendor_id: u16,
    device_id: u16,
    codec: u8,
    dac: u8,
    pin: u8,
    stream_base: usize,
    write_period: usize,
    filled_periods: usize,
    last_hw_period: usize,
    frames_played: u64,
    last_lpib: u32,
    underruns: u32,
    running: bool,
}

// SAFETY: audiod is a single-threaded service. The MMIO/DMA pointers are
// exclusively owned by this process and never shared across threads.
unsafe impl Send for HdaPlayback {}

impl HdaPlayback {
    pub fn open() -> Result<Self, HdaError> {
        let info = hda_info().ok_or(HdaError::NoDevice)?;
        if info.bar_phys == 0 || info.bar_size < 0x100 || info.bar_size > 1024 * 1024 {
            return Err(HdaError::BadResource);
        }
        let mmio = map_mmio(info.bar_phys, info.bar_size).ok_or(HdaError::MapFailed)?;
        let (dma, dma_phys) = dma_alloc(DMA_PAGES).ok_or(HdaError::DmaFailed)?;
        if dma_phys == 0 || (dma_phys & 0xfff) != 0 {
            return Err(HdaError::DmaFailed);
        }
        unsafe {
            core::ptr::write_bytes(dma, 0, DMA_PAGES * PAGE);
        }
        let mut dev = Self {
            mmio,
            dma,
            dma_phys,
            bar_phys: info.bar_phys,
            bar_size: info.bar_size,
            vendor_id: info.vendor_id,
            device_id: info.device_id,
            codec: 0,
            dac: 0,
            pin: 0,
            stream_base: 0,
            write_period: 0,
            filled_periods: 0,
            last_hw_period: 0,
            frames_played: 0,
            last_lpib: 0,
            underruns: 0,
            running: false,
        };
        dev.reset_controller()?;
        dev.codec = dev.wait_for_codec().ok_or(HdaError::NoCodec)?;
        dev.setup_rings()?;
        dev.discover_output()?;
        dev.configure_path()?;
        dev.setup_stream()?;
        Ok(dev)
    }

    pub fn capabilities(&self) -> AudioCapabilities {
        AudioCapabilities {
            kind: OutputDeviceKind::from_pci(self.vendor_id, self.device_id),
            vendor_id: self.vendor_id,
            device_id: self.device_id,
            max_channels: 2,
            sample_bits: 16,
            native_rate_hz: NATIVE_RATE_HZ,
            playback: true,
        }
    }

    pub fn state(&self) -> AudioDeviceState {
        if self.running {
            AudioDeviceState::Playing
        } else {
            AudioDeviceState::Ready
        }
    }

    pub fn frames_played(&self) -> u64 {
        self.frames_played
    }

    pub fn underruns(&self) -> u32 {
        self.underruns
    }

    pub fn bar_phys(&self) -> u64 {
        self.bar_phys
    }

    pub fn bar_size(&self) -> u64 {
        self.bar_size
    }

    pub fn start(&mut self) -> Result<(), HdaError> {
        if self.running {
            return Ok(());
        }
        self.write_period = 0;
        self.filled_periods = 0;
        unsafe {
            core::ptr::write_bytes(self.dma.add(PAGE), 0, RING_BYTES);
        }
        unsafe {
            self.w32(self.stream_base + 0x08, RING_BYTES as u32);
            self.w16(self.stream_base + 0x0c, (PERIODS as u16) - 1);
            self.w16(self.stream_base + 0x12, HDA_FMT_48K_S16_STEREO);
            self.w32(
                self.stream_base + 0x18,
                (self.bdl_phys() & 0xffff_ffff) as u32,
            );
            self.w32(self.stream_base + 0x1c, (self.bdl_phys() >> 32) as u32);
            let ctl = self.r32(self.stream_base) & !0x00ff_ffff;
            let stream = (STREAM_ID as u32) << 20;
            self.w32(self.stream_base, ctl | stream | 0x2);
        }
        self.last_lpib = self.dma_position_bytes();
        self.last_hw_period = (self.last_lpib as usize / PERIOD_BYTES) % PERIODS;
        self.running = true;
        Ok(())
    }

    pub fn stop(&mut self) {
        if !self.running {
            return;
        }
        unsafe {
            let ctl = self.r32(self.stream_base);
            self.w32(self.stream_base, ctl & !0x2);
            self.w8(self.stream_base + 0x03, 0x1c);
        }
        self.running = false;
    }

    /// Drop every hardware-resident period and restart with silence. This is
    /// the output-side flush required by pause, stop, and media seek; clearing
    /// audiod's producer queue alone cannot retract DMA already submitted.
    pub fn flush(&mut self) -> Result<(), HdaError> {
        // Preserve progress up to the point at which the stream is stopped.
        // `frames_played` is a hardware-consumed clock, never a submit clock.
        let _ = self.poll_dma_progress();
        self.stop();
        self.start()?;
        let _ = self.fill_silence_ready()?;
        Ok(())
    }

    /// Whether one full engine period can be submitted without overwriting a
    /// period the controller has not consumed yet.
    pub fn can_submit_period(&self) -> bool {
        self.filled_periods < PERIODS
    }

    /// Copy `src` (already gain-applied S16LE stereo) into the next free period.
    /// Returns the number of periods filled.
    pub fn submit_period(&mut self, src: &[u8]) -> Result<bool, HdaError> {
        if !self.running {
            self.start()?;
        }
        if self.filled_periods >= PERIODS {
            return Ok(false);
        }
        let n = src.len().min(PERIOD_BYTES);
        unsafe {
            let dst = self.dma.add(PAGE + self.write_period * PERIOD_BYTES);
            core::ptr::write_bytes(dst, 0, PERIOD_BYTES);
            if n > 0 {
                core::ptr::copy_nonoverlapping(src.as_ptr(), dst, n);
            }
        }
        self.write_period = (self.write_period + 1) % PERIODS;
        self.filled_periods += 1;
        self.ack_stream();
        Ok(true)
    }

    pub fn fill_silence_ready(&mut self) -> Result<u8, HdaError> {
        let mut filled = 0u8;
        for _ in 0..PERIODS {
            if !self.submit_period(&[])? {
                break;
            }
            filled = filled.saturating_add(1);
        }
        if filled == 0 {
            self.underruns = self.underruns.saturating_add(1);
        }
        Ok(filled)
    }

    pub fn fill_sine(
        &mut self,
        phase: &mut u32,
        freq_hz: u32,
        volume: u8,
    ) -> Result<bool, HdaError> {
        let mut tmp = [0u8; PERIOD_BYTES];
        let (next, _) =
            generate_sine_s16le_stereo(&mut tmp, *phase, freq_hz, NATIVE_RATE_HZ, volume);
        *phase = next;
        self.submit_period(&tmp)
    }

    pub fn fill_pcm(&mut self, pcm: &[u8], volume: u8) -> Result<bool, HdaError> {
        let mut tmp = [0u8; PERIOD_BYTES];
        let n = pcm.len().min(PERIOD_BYTES);
        tmp[..n].copy_from_slice(&pcm[..n]);
        apply_gain_s16le(&mut tmp[..n], volume);
        self.submit_period(&tmp)
    }

    pub fn position_frames(&self) -> u32 {
        self.dma_position_bytes() / FRAME_BYTES as u32
    }

    /// Sample LPIB and account hardware-consumed frames across ring wraps.
    ///
    /// This intentionally measures controller progress, including silence,
    /// rather than bytes most recently submitted by a client.
    pub fn poll_dma_progress(&mut self) -> u64 {
        if !self.running {
            return self.frames_played;
        }
        let current = self.dma_position_bytes() % RING_BYTES as u32;
        let previous = self.last_lpib % RING_BYTES as u32;
        let advanced = ring_advance_bytes(previous, current, RING_BYTES as u32);
        self.frames_played = self
            .frames_played
            .saturating_add((advanced / FRAME_BYTES as u32) as u64);
        let current_period = (current as usize / PERIOD_BYTES) % PERIODS;
        let periods_advanced = ring_advance_periods(self.last_hw_period, current_period, PERIODS);
        if periods_advanced != 0 {
            self.filled_periods = self
                .filled_periods
                .saturating_sub(periods_advanced.min(PERIODS));
            self.last_hw_period = current_period;
        }
        self.last_lpib = current;
        self.frames_played
    }

    fn dma_position_bytes(&self) -> u32 {
        unsafe { self.r32(self.stream_base + 0x04) }
    }

    fn ack_stream(&mut self) {
        unsafe {
            let sts = self.r8(self.stream_base + 0x03);
            if sts != 0 {
                self.w8(self.stream_base + 0x03, sts);
            }
            let ints = self.r32(INTSTS);
            if ints != 0 {
                self.w32(INTSTS, ints);
            }
        }
    }

    fn reset_controller(&mut self) -> Result<(), HdaError> {
        unsafe {
            self.w32(GCTL, 0);
            if !self.wait_mask_u32(GCTL, GCTL_CRST, 0, RESET_TIMEOUT_MS) {
                return Err(HdaError::ResetTimeout);
            }
            self.w32(GCTL, GCTL_CRST);
            if !self.wait_mask_u32(GCTL, GCTL_CRST, GCTL_CRST, RESET_TIMEOUT_MS) {
                return Err(HdaError::ResetTimeout);
            }
            self.w32(INTCTL, 0);
        }
        Ok(())
    }

    fn wait_for_codec(&self) -> Option<u8> {
        // STATESTS is write-one-to-clear. Do not acknowledge it before
        // latching codec presence or the codec becomes invisible to discovery.
        // A codec may also need a short time after CRST is asserted to report.
        let start = monotonic_millis();
        loop {
            let state = unsafe { self.r16(STATESTS) };
            if let Some(codec) = first_codec_in_status(state) {
                return Some(codec);
            }
            if monotonic_millis().saturating_sub(start) >= CODEC_DETECT_TIMEOUT_MS {
                return None;
            }
            process_yield();
        }
    }

    fn setup_rings(&mut self) -> Result<(), HdaError> {
        unsafe {
            self.w8(CORBCTL, 0);
            self.w8(RIRBCTL, 0);
            self.w8(CORBSIZE, 0x02);
            self.w8(RIRBSIZE, 0x02);
            self.w32(CORBLBASE, (self.corb_phys() & 0xffff_ffff) as u32);
            self.w32(CORBUBASE, (self.corb_phys() >> 32) as u32);
            self.w32(RIRBLBASE, (self.rirb_phys() & 0xffff_ffff) as u32);
            self.w32(RIRBUBASE, (self.rirb_phys() >> 32) as u32);
            self.w16(CORBRP, 0x8000);
            let _ = self.wait_mask_u16(CORBRP, 0x8000, 0x8000, CMD_TIMEOUT_MS);
            self.w16(CORBRP, 0);
            self.w16(CORBWP, 0);
            self.w16(RIRBWP, 0x8000);
            self.w16(RINTCNT, 1);
            self.w8(CORBCTL, 0x02);
            self.w8(RIRBCTL, 0x02);
        }
        Ok(())
    }

    fn discover_output(&mut self) -> Result<(), HdaError> {
        let vendor = self.verb(0, VERB_GET_PARAM, PARAM_VENDOR)?;
        let _ = vendor;
        let sub = self.verb(0, VERB_GET_PARAM, PARAM_SUBORD)?;
        let start = ((sub >> 16) & 0xff) as u8;
        let count = (sub & 0xff) as u8;
        if count == 0 || count as usize > MAX_WIDGETS {
            return self.fallback_path();
        }
        let mut afg = start;
        let mut dac = 0u8;
        let mut pin = 0u8;
        for nid in start..start.saturating_add(count) {
            if let Ok(fn_type) = self.verb(nid, VERB_GET_PARAM, PARAM_FNGROUP) {
                if (fn_type & 0xff) == 0x01 {
                    afg = nid;
                }
            }
            if let Ok(cap) = self.verb(nid, VERB_GET_PARAM, PARAM_WIDGET_CAP) {
                let wtype = (cap >> 20) & 0xf;
                if wtype == WIDGET_AUDIO_OUT && dac == 0 {
                    dac = nid;
                }
                if wtype == WIDGET_PIN && pin == 0 {
                    if let Ok(pc) = self.verb(nid, VERB_GET_PARAM, PARAM_PIN_CAP) {
                        if pc & (1 << 4) != 0 {
                            pin = nid;
                        }
                    }
                }
            }
        }
        if dac == 0 || pin == 0 {
            return self.fallback_path();
        }
        let _ = afg;
        self.dac = dac;
        self.pin = pin;
        Ok(())
    }

    fn fallback_path(&mut self) -> Result<(), HdaError> {
        // QEMU hda-output: AFG=1, DAC=2, PIN=3.
        if self.verb(2, VERB_GET_PARAM, PARAM_WIDGET_CAP).is_ok() {
            self.dac = 2;
            self.pin = 3;
            return Ok(());
        }
        Err(HdaError::NoOutputPath)
    }

    fn configure_path(&mut self) -> Result<(), HdaError> {
        let _ = self.verb(1, VERB_SET_POWER, 0);
        let _ = self.verb(self.dac, VERB_SET_POWER, 0);
        let _ = self.verb(self.pin, VERB_SET_POWER, 0);
        let _ = self.verb(
            self.dac,
            VERB_SET_STREAM_FORMAT,
            HDA_FMT_48K_S16_STEREO as u32,
        );
        let _ = self.verb(self.dac, VERB_SET_CHAN_STREAMID, (STREAM_ID as u32) << 4);
        // The amplifier gain field is an index, not a signed dB value. The
        // capability offset identifies the step representing 0 dB. Writing
        // gain index 0 silences QEMU's codec (whose 0 dB offset is 0x4a).
        if let Ok(amp_caps) = self.verb(self.dac, VERB_GET_PARAM, PARAM_AMP_OUT_CAP) {
            let zero_db_gain = amp_zero_db_gain(amp_caps);
            let _ = self.verb(self.dac, VERB_SET_AMP_GAIN, 0xb000 | zero_db_gain);
        }
        let _ = self.verb(self.pin, VERB_SET_PIN_WIDGET, 0x40);
        let _ = self.verb(self.pin, VERB_SET_EAPD, 0x02);
        let _ = self.verb(self.pin, VERB_GET_PARAM, PARAM_CONN_LIST_LEN);
        Ok(())
    }

    fn setup_stream(&mut self) -> Result<(), HdaError> {
        let gcap = unsafe { self.r16(0x00) };
        let iss = ((gcap >> 8) & 0xf) as usize;
        self.stream_base = 0x80 + iss * 0x20;
        if self.stream_base + 0x20 > self.bar_size as usize {
            return Err(HdaError::BadResource);
        }
        unsafe {
            self.w32(self.stream_base, 0);
            self.w8(self.stream_base + 0x03, 0x1c);
        }
        self.install_bdl();
        Ok(())
    }

    fn install_bdl(&mut self) {
        unsafe {
            let bdl = self.dma.add(self.bdl_off()) as *mut BdlEntry;
            for i in 0..PERIODS {
                let entry = BdlEntry {
                    addr: self.pcm_phys(i),
                    length: PERIOD_BYTES as u32,
                    flags: 1, // IOC
                };
                core::ptr::write_volatile(bdl.add(i), entry);
            }
        }
    }

    fn verb(&mut self, nid: u8, verb: u32, payload: u32) -> Result<u32, HdaError> {
        let cmd = ((self.codec as u32) << 28)
            | ((nid as u32) << 20)
            | ((verb & 0xfff) << 8)
            | (payload & 0xff);
        let wide = if verb < 0x10 {
            ((self.codec as u32) << 28)
                | ((nid as u32) << 20)
                | ((verb & 0xf) << 16)
                | (payload & 0xffff)
        } else {
            cmd
        };
        match self.immediate_verb(wide) {
            Ok(response) => Ok(response),
            Err(_) => self.corb_verb(wide),
        }
    }

    fn immediate_verb(&mut self, cmd: u32) -> Result<u32, HdaError> {
        let start = monotonic_millis();
        unsafe {
            if self.r16(ICIS) & ICIS_ICB != 0 {
                return Err(HdaError::CodecTimeout);
            }
            self.w32(ICOI, cmd);
            self.w16(ICIS, ICIS_ICB);
            loop {
                let s = self.r16(ICIS);
                if s & ICIS_ICB == 0 {
                    if s & ICIS_IRV == 0 {
                        return Err(HdaError::CodecTimeout);
                    }
                    return Ok(self.r32(ICII));
                }
                if monotonic_millis().saturating_sub(start) > CMD_TIMEOUT_MS {
                    return Err(HdaError::CodecTimeout);
                }
            }
        }
    }

    fn corb_verb(&mut self, cmd: u32) -> Result<u32, HdaError> {
        unsafe {
            let wp = self.r16(CORBWP) as usize;
            let next = (wp + 1) % CORB_ENTRIES;
            let slot = self.dma.add(self.corb_off() + next * 4) as *mut u32;
            core::ptr::write_volatile(slot, cmd);
            self.w16(CORBWP, next as u16);
            let start = monotonic_millis();
            loop {
                let sts = self.r8(RIRBSTS);
                if sts & 0x1 != 0 {
                    self.w8(RIRBSTS, sts);
                    let rwp = self.r16(RIRBWP) as usize & 0xff;
                    let resp = self.dma.add(self.rirb_off() + rwp * 8) as *const u32;
                    return Ok(core::ptr::read_volatile(resp));
                }
                if monotonic_millis().saturating_sub(start) > CMD_TIMEOUT_MS {
                    return Err(HdaError::CodecTimeout);
                }
            }
        }
    }

    fn corb_off(&self) -> usize {
        0
    }
    fn rirb_off(&self) -> usize {
        CORB_ENTRIES * 4
    }
    fn bdl_off(&self) -> usize {
        let raw = self.rirb_off() + RIRB_ENTRIES * 8;
        (raw + BDL_ALIGN - 1) & !(BDL_ALIGN - 1)
    }
    fn corb_phys(&self) -> u64 {
        self.dma_phys + self.corb_off() as u64
    }
    fn rirb_phys(&self) -> u64 {
        self.dma_phys + self.rirb_off() as u64
    }
    fn bdl_phys(&self) -> u64 {
        self.dma_phys + self.bdl_off() as u64
    }
    fn pcm_phys(&self, period: usize) -> u64 {
        self.dma_phys + PAGE as u64 + (period * PERIOD_BYTES) as u64
    }

    unsafe fn r8(&self, off: usize) -> u8 {
        core::ptr::read_volatile(self.mmio.add(off))
    }
    unsafe fn r16(&self, off: usize) -> u16 {
        core::ptr::read_volatile(self.mmio.add(off) as *const u16)
    }
    unsafe fn r32(&self, off: usize) -> u32 {
        core::ptr::read_volatile(self.mmio.add(off) as *const u32)
    }
    unsafe fn w8(&self, off: usize, v: u8) {
        core::ptr::write_volatile(self.mmio.add(off), v);
    }
    unsafe fn w16(&self, off: usize, v: u16) {
        core::ptr::write_volatile(self.mmio.add(off) as *mut u16, v);
    }
    unsafe fn w32(&self, off: usize, v: u32) {
        core::ptr::write_volatile(self.mmio.add(off) as *mut u32, v);
    }

    fn wait_mask_u32(&self, off: usize, mask: u32, expect: u32, timeout_ms: u64) -> bool {
        let start = monotonic_millis();
        loop {
            if unsafe { self.r32(off) } & mask == expect {
                return true;
            }
            if monotonic_millis().saturating_sub(start) > timeout_ms {
                return false;
            }
        }
    }

    fn wait_mask_u16(&self, off: usize, mask: u16, expect: u16, timeout_ms: u64) -> bool {
        let start = monotonic_millis();
        loop {
            if unsafe { self.r16(off) } & mask == expect {
                return true;
            }
            if monotonic_millis().saturating_sub(start) > timeout_ms {
                return false;
            }
        }
    }
}

impl Drop for HdaPlayback {
    fn drop(&mut self) {
        self.stop();
    }
}

pub const PERIOD_FRAME_COUNT: usize = PERIOD_BYTES / FRAME_BYTES;
pub const ENGINE_PERIOD_BYTES: usize = PERIOD_BYTES;

const fn first_codec_in_status(state: u16) -> Option<u8> {
    let mut codec = 0u8;
    while codec < 15 {
        if state & (1u16 << codec) != 0 {
            return Some(codec);
        }
        codec += 1;
    }
    None
}

const fn ring_advance_bytes(previous: u32, current: u32, ring_bytes: u32) -> u32 {
    if current >= previous {
        current - previous
    } else {
        ring_bytes - previous + current
    }
}

const fn ring_advance_periods(previous: usize, current: usize, periods: usize) -> usize {
    (current + periods - previous) % periods
}

const fn amp_zero_db_gain(amp_caps: u32) -> u32 {
    amp_caps & 0x7f
}

#[cfg(test)]
mod tests {
    use super::{
        amp_zero_db_gain, first_codec_in_status, ring_advance_bytes, ring_advance_periods,
        RING_BYTES,
    };

    #[test]
    fn codec_presence_selects_lowest_codec_address() {
        assert_eq!(first_codec_in_status(0), None);
        assert_eq!(first_codec_in_status(1), Some(0));
        assert_eq!(first_codec_in_status(0b1010), Some(1));
        assert_eq!(first_codec_in_status(1 << 14), Some(14));
    }

    #[test]
    fn dma_progress_accounts_for_ring_wrap() {
        assert_eq!(ring_advance_bytes(100, 220, RING_BYTES as u32), 120);
        assert_eq!(
            ring_advance_bytes(RING_BYTES as u32 - 64, 128, RING_BYTES as u32),
            192
        );
    }

    #[test]
    fn period_progress_distinguishes_full_ring_from_empty_ring() {
        assert_eq!(ring_advance_periods(0, 0, 4), 0);
        assert_eq!(ring_advance_periods(0, 1, 4), 1);
        assert_eq!(ring_advance_periods(3, 0, 4), 1);
    }

    #[test]
    fn output_amp_offset_is_the_zero_db_gain_index() {
        assert_eq!(amp_zero_db_gain(0x8000_4a4a), 0x4a);
        assert_eq!(amp_zero_db_gain(0x8000_3217), 0x17);
    }
}
