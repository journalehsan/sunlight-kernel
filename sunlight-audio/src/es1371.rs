//! Ensoniq AudioPCI ES1371 playback backend.
//!
//! The kernel owns PCI discovery, command-bit preservation, port permission,
//! IRQ metadata, and contiguous DMA allocation. This module only receives a
//! validated I/O-port grant and programs Playback 2 with S16LE stereo PCM.

use crate::hda::DmaProgress;
use crate::pcm::{apply_gain_s16le, generate_sine_s16le_stereo, FRAME_BYTES, NATIVE_RATE_HZ};
use crate::{AudioCapabilities, AudioDeviceState, AudioError};
use sunlight_ipc::{
    dma_alloc, es1371_info, io_port_read, io_port_write, monotonic_millis, process_yield,
};

const PERIODS: usize = 12;
const PERIOD_BYTES: usize = 4096;
const RING_BYTES: usize = PERIODS * PERIOD_BYTES;
const DMA_PAGES: usize = 1 + PERIODS;
const DMA_OFFSET: usize = 4096;
const REG_CONTROL: u16 = 0x00;
const REG_STATUS: u16 = 0x04;
const STATUS_DAC2: u32 = 1 << 1;
const REG_MEM_PAGE: u16 = 0x0c;
const REG_SRC: u16 = 0x10;
const REG_CODEC: u16 = 0x14;
const REG_SERIAL: u16 = 0x20;
const REG_DAC2_COUNT: u16 = 0x28;
const REG_DAC2_FRAME: u16 = 0x38;
const REG_DAC2_SIZE: u16 = 0x3c;
const CTRL_SYNC_RES: u32 = 1 << 14;
const CTRL_BREQ: u32 = 1 << 7;
const CTRL_DAC2_EN: u32 = 1 << 5;
const CODEC_RDY: u32 = 1 << 31;
const CODEC_WIP: u32 = 1 << 30;
const CODEC_PIRD: u32 = 1 << 23;
const SRC_BUSY: u32 = 1 << 23;
const SRC_WE: u32 = 1 << 24;
const SRC_DISABLE: u32 = 1 << 22;
const SRC_DIS_P1: u32 = 1 << 21;
const SRC_DIS_P2: u32 = 1 << 20;
const SRC_DIS_R1: u32 = 1 << 19;
const SRC_SAFE: u32 = 1 << 16;
const SRC_STATE_MASK: u32 = SRC_DISABLE | SRC_DIS_P1 | SRC_DIS_P2 | SRC_DIS_R1;
const SRC_DAC2: u16 = 0x74;
const SRC_INT_REGS: u16 = 1;
const SRC_VFREQ_FRAC: u16 = 3;
const SCTRL_P2_INT_EN: u32 = 1 << 9;
const SCTRL_P2_END_INC: u32 = 2 << 19;
const SCTRL_P2_MODE_S16_STEREO: u32 = 3 << 2;

/// Polling replaces interrupt delivery, but must still acknowledge the device's
/// period latch. Only toggle P2_INT_EN; rewriting the count or DMA registers
/// here would restart playback rather than retire a completed period.
fn poll_playback2(
    serial: u32,
    read: impl Fn(u16) -> Result<u32, Es1371Error>,
    write: impl Fn(u16, u32) -> Result<(), Es1371Error>,
) -> Result<(u32, bool), Es1371Error> {
    let status = read(REG_STATUS)?;
    let position = (read(REG_DAC2_SIZE)? >> 14) & 0x3fffc;
    let pending = status & STATUS_DAC2 != 0;
    if pending {
        write(REG_SERIAL, serial & !SCTRL_P2_INT_EN)?;
        write(REG_SERIAL, serial)?;
    }
    Ok((position, pending))
}

#[derive(Clone, Copy, Debug)]
pub struct Es1371DmaRegisters {
    pub control: u32,
    pub serial: u32,
    pub size: u32,
    pub count: u32,
    pub status: u32,
    pub acknowledgements: u64,
    pub io_failed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Es1371Error {
    NoDevice,
    BadResource,
    Dma,
    Timeout,
    Io,
    Failed,
}
impl Es1371Error {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoDevice => "no-device",
            Self::BadResource => "invalid-io-resource",
            Self::Dma => "dma-allocation-failed",
            Self::Timeout => "hardware-timeout",
            Self::Io => "io-access-failed",
            Self::Failed => "hardware-failed",
        }
    }
}
impl From<Es1371Error> for AudioError {
    fn from(e: Es1371Error) -> Self {
        match e {
            Es1371Error::NoDevice => Self::DeviceUnavailable,
            Es1371Error::Timeout => Self::Timeout,
            _ => Self::DeviceFailed,
        }
    }
}

pub struct Es1371Playback {
    port: u16,
    dma: *mut u8,
    dma_phys: u64,
    vendor_id: u16,
    device_id: u16,
    irq: u8,
    control: u32,
    serial: u32,
    write_period: usize,
    last_hw_period: usize,
    filled_periods: usize,
    last_position: u32,
    frames_played: u64,
    underruns: u32,
    running: bool,
    acknowledgements: u64,
    io_failed: bool,
}
unsafe impl Send for Es1371Playback {}

/// Pure BAR validation shared by host-side tests and the hardware contract.
pub const fn validate_io_bar(raw: u32, mask: u32) -> Option<(u16, u32)> {
    if raw & 1 == 0 {
        return None;
    }
    let base = raw & !0x3;
    let size = (!(mask & !0x3)).wrapping_add(1);
    if base == 0
        || size < 0x40
        || size > 0x1000
        || !size.is_power_of_two()
        || base & (size - 1) != 0
    {
        return None;
    }
    if base as u64 + size as u64 > 0x1_0000 {
        return None;
    }
    Some((base as u16, size))
}

impl Es1371Playback {
    pub fn open() -> Result<Self, Es1371Error> {
        let info = es1371_info().ok_or(Es1371Error::NoDevice)?;
        if info.io_size < 0x40 || info.io_size > 0x1000 || info.io_port == 0 {
            return Err(Es1371Error::BadResource);
        }
        let (dma, dma_phys) = dma_alloc(DMA_PAGES).ok_or(Es1371Error::Dma)?;
        if dma_phys == 0
            || dma_phys & 0xfff != 0
            || dma_phys > u32::MAX as u64
            || dma_phys.saturating_add((DMA_OFFSET + RING_BYTES) as u64) > u32::MAX as u64 + 1
        {
            return Err(Es1371Error::Dma);
        }
        unsafe {
            core::ptr::write_bytes(dma, 0, DMA_PAGES * 4096);
        }
        let mut dev = Self {
            port: info.io_port,
            dma,
            dma_phys,
            vendor_id: info.vendor_id,
            device_id: info.device_id,
            irq: info.irq,
            // BREQ enables the controller's memory bus requests. Keep it
            // asserted for the lifetime of the playback engine; it is not a
            // controller test bit.
            control: CTRL_BREQ,
            serial: 0,
            write_period: 0,
            last_hw_period: 0,
            filled_periods: 0,
            last_position: 0,
            frames_played: 0,
            underruns: 0,
            running: false,
            acknowledgements: 0,
            io_failed: false,
        };
        dev.reset()?;
        dev.init_codec()?;
        dev.configure_ring()?;
        dev.init_rate()?;
        Ok(dev)
    }

    pub fn capabilities(&self) -> AudioCapabilities {
        AudioCapabilities::es1371(self.vendor_id, self.device_id)
    }
    pub fn irq(&self) -> u8 {
        self.irq
    }
    pub fn state(&self) -> AudioDeviceState {
        if self.io_failed {
            AudioDeviceState::Failed
        } else if self.running {
            AudioDeviceState::Playing
        } else if self.underruns != 0 {
            AudioDeviceState::Underrun
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
    pub fn position_frames(&self) -> u32 {
        (self.last_position / FRAME_BYTES as u32) % (RING_BYTES as u32 / FRAME_BYTES as u32)
    }
    /// Read-only register snapshot used to diagnose a stalled VMware or
    /// physical ES1371 engine. The caller must keep this out of IRQ context.
    pub fn dma_registers(&self) -> Es1371DmaRegisters {
        Es1371DmaRegisters {
            control: self.read32(REG_CONTROL).unwrap_or(u32::MAX),
            serial: self.read32(REG_SERIAL).unwrap_or(u32::MAX),
            size: self.read32(REG_DAC2_SIZE).unwrap_or(u32::MAX),
            count: self.read32(REG_DAC2_COUNT).unwrap_or(u32::MAX),
            status: self.read32(REG_STATUS).unwrap_or(u32::MAX),
            acknowledgements: self.acknowledgements,
            io_failed: self.io_failed,
        }
    }
    pub fn input_available(&self) -> bool {
        false
    }
    pub fn capture_lost_periods(&self) -> u64 {
        0
    }
    pub fn start_capture(&mut self) -> Result<(), Es1371Error> {
        Err(Es1371Error::Failed)
    }
    pub fn stop_capture(&mut self) {}

    fn read32(&self, offset: u16) -> Result<u32, Es1371Error> {
        io_port_read(self.port.checked_add(offset).ok_or(Es1371Error::Io)?, 4)
            .ok_or(Es1371Error::Io)
    }
    fn write32(&self, offset: u16, value: u32) -> Result<(), Es1371Error> {
        io_port_write(
            self.port.checked_add(offset).ok_or(Es1371Error::Io)?,
            4,
            value,
        )
        .then_some(())
        .ok_or(Es1371Error::Io)
    }
    fn wait_until(&self, mut ready: impl FnMut(u32) -> bool) -> Result<u32, Es1371Error> {
        let deadline = monotonic_millis().saturating_add(50);
        loop {
            let value = self.read32(REG_SRC)?;
            if ready(value) {
                return Ok(value);
            }
            if monotonic_millis() >= deadline {
                return Err(Es1371Error::Timeout);
            }
            process_yield();
        }
    }
    fn reset(&mut self) -> Result<(), Es1371Error> {
        self.control = CTRL_BREQ;
        self.serial = 0;
        self.write32(REG_CONTROL, self.control | CTRL_SYNC_RES)?;
        let deadline = monotonic_millis().saturating_add(10);
        while monotonic_millis() < deadline {
            process_yield();
        }
        self.write32(REG_CONTROL, self.control)?;
        self.write32(REG_SERIAL, 0)?;
        Ok(())
    }
    fn init_codec(&mut self) -> Result<(), Es1371Error> {
        self.write32(REG_CONTROL, self.control | CTRL_SYNC_RES)?;
        self.write32(REG_CONTROL, self.control)?;
        self.codec_write(0x02, 0)?;
        self.codec_write(0x18, 0)?;
        Ok(())
    }
    fn init_rate(&mut self) -> Result<(), Es1371Error> {
        let freq = ((NATIVE_RATE_HZ as u64 * (1 << 15)) / 3000) as u32;
        let disabled = self.wait_until(|v| v & SRC_BUSY == 0)? & SRC_STATE_MASK & !SRC_DIS_P2;
        self.write32(REG_SRC, disabled | SRC_DIS_P2)?;
        // Some VMware revisions do not expose the SRC readback phase exactly
        // like physical ES1371 silicon. Preserve the low control byte when it
        // is available, but keep the documented rate programming path usable
        // if that optional readback times out.
        let int_regs = self.src_read(SRC_DAC2 + SRC_INT_REGS).unwrap_or(0) & 0x00ff;
        self.src_write(SRC_DAC2 + SRC_INT_REGS, int_regs | ((freq >> 5) & 0xfc00))?;
        self.src_write(SRC_DAC2 + SRC_VFREQ_FRAC, freq & 0x7fff)?;
        let disabled = self.wait_until(|v| v & SRC_BUSY == 0)? & SRC_STATE_MASK & !SRC_DIS_P2;
        self.write32(REG_SRC, disabled)
    }
    fn configure_ring(&mut self) -> Result<(), Es1371Error> {
        self.write32(REG_MEM_PAGE, 0x0c)?;
        self.write32(REG_DAC2_FRAME, (self.dma_phys + DMA_OFFSET as u64) as u32)?;
        self.write32(REG_DAC2_SIZE, (RING_BYTES as u32 / 4) - 1)?;
        // Keep the device's period interrupt source enabled as required by
        // the ES1371 playback setup. audiod still polls the DMA cursor because
        // the generic PCI-audio IRQ wait/ack endpoint is not exposed yet.
        self.serial = SCTRL_P2_INT_EN | SCTRL_P2_END_INC | SCTRL_P2_MODE_S16_STEREO;
        self.write32(REG_SERIAL, self.serial)?;
        // DAC2_COUNT defines the hardware period. It is initialized once;
        // rewriting it while the engine is running resets the current sample
        // counter on ES1371 and prevents DMA progress.
        self.write32(REG_DAC2_COUNT, (PERIOD_BYTES as u32 / 4) - 1)
    }
    fn src_write(&self, register: u16, data: u32) -> Result<(), Es1371Error> {
        let state = self.wait_until(|v| v & SRC_BUSY == 0)? & SRC_STATE_MASK;
        self.write32(
            REG_SRC,
            state | ((register as u32 & 0x7f) << 25) | SRC_WE | data,
        )
    }
    fn src_read(&self, register: u16) -> Result<u32, Es1371Error> {
        let original = self.wait_until(|v| v & SRC_BUSY == 0)?;
        let state = original & SRC_STATE_MASK;
        self.write32(REG_SRC, state | ((register as u32 & 0x7f) << 25) | SRC_SAFE)?;
        let deadline = monotonic_millis().saturating_add(50);
        loop {
            let value = self.read32(REG_SRC)?;
            if value & 0x0087_0000 == SRC_SAFE {
                let data = value & 0xffff;
                self.write32(REG_SRC, state)?;
                return Ok(data);
            }
            if monotonic_millis() >= deadline {
                return Err(Es1371Error::Timeout);
            }
            process_yield();
        }
    }
    fn codec_write(&self, register: u16, data: u16) -> Result<(), Es1371Error> {
        let deadline = monotonic_millis().saturating_add(50);
        loop {
            if self.read32(REG_CODEC)? & CODEC_WIP == 0 {
                self.write32(REG_CODEC, ((register as u32 & 0x7f) << 16) | data as u32)?;
                return Ok(());
            }
            if monotonic_millis() >= deadline {
                return Err(Es1371Error::Timeout);
            }
            process_yield();
        }
    }
    #[allow(dead_code)]
    fn codec_read(&self, register: u16) -> Result<u16, Es1371Error> {
        self.wait_codec_ready()?;
        self.write32(REG_CODEC, ((register as u32 & 0x7f) << 16) | CODEC_PIRD)?;
        let deadline = monotonic_millis().saturating_add(50);
        loop {
            let value = self.read32(REG_CODEC)?;
            if value & CODEC_RDY != 0 {
                return Ok(value as u16);
            }
            if monotonic_millis() >= deadline {
                return Err(Es1371Error::Timeout);
            }
            process_yield();
        }
    }
    fn wait_codec_ready(&self) -> Result<(), Es1371Error> {
        let deadline = monotonic_millis().saturating_add(50);
        while self.read32(REG_CODEC)? & CODEC_WIP != 0 {
            if monotonic_millis() >= deadline {
                return Err(Es1371Error::Timeout);
            }
            process_yield();
        }
        Ok(())
    }

    pub fn start(&mut self) -> Result<(), Es1371Error> {
        if self.io_failed {
            return Err(Es1371Error::Io);
        }
        if !self.running {
            self.control |= CTRL_DAC2_EN;
            self.write32(REG_CONTROL, self.control)?;
            self.running = true;
        }
        Ok(())
    }
    pub fn stop(&mut self) {
        self.control &= !CTRL_DAC2_EN;
        let _ = self.write32(REG_CONTROL, self.control);
        self.running = false;
        self.filled_periods = 0;
    }
    pub fn flush_with_progress(&mut self) -> Result<DmaProgress, Es1371Error> {
        let progress = self.poll_dma_progress_report();
        self.stop();
        self.start()?;
        let _ = self.fill_silence_ready()?;
        Ok(progress)
    }
    pub fn can_submit_period(&self) -> bool {
        self.filled_periods < PERIODS - 1
    }
    pub fn last_submitted_period(&self) -> usize {
        (self.write_period + PERIODS - 1) % PERIODS
    }
    pub fn fill_silence_ready(&mut self) -> Result<u8, Es1371Error> {
        let mut count = 0;
        while self.filled_periods < 2 {
            if !self.submit_period(&[])? {
                break;
            }
            count += 1;
        }
        Ok(count)
    }
    pub fn prime_free_silence(&mut self) {
        for offset in 0..PERIODS.saturating_sub(self.filled_periods) {
            unsafe {
                core::ptr::write_bytes(
                    self.dma
                        .add(DMA_OFFSET + ((self.write_period + offset) % PERIODS) * PERIOD_BYTES),
                    0,
                    PERIOD_BYTES,
                );
            }
        }
    }
    fn submit_period(&mut self, src: &[u8]) -> Result<bool, Es1371Error> {
        if !self.running {
            self.start()?;
        }
        if !self.can_submit_period() {
            return Ok(false);
        }
        let dst = unsafe {
            core::slice::from_raw_parts_mut(
                self.dma.add(DMA_OFFSET + self.write_period * PERIOD_BYTES),
                PERIOD_BYTES,
            )
        };
        dst.fill(0);
        dst[..src.len().min(PERIOD_BYTES)].copy_from_slice(&src[..src.len().min(PERIOD_BYTES)]);
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        self.write_period = (self.write_period + 1) % PERIODS;
        self.filled_periods += 1;
        Ok(true)
    }
    pub fn fill_pcm(&mut self, pcm: &[u8], volume: u8) -> Result<bool, Es1371Error> {
        let mut tmp = [0u8; PERIOD_BYTES];
        let n = pcm.len().min(PERIOD_BYTES);
        tmp[..n].copy_from_slice(&pcm[..n]);
        apply_gain_s16le(&mut tmp[..n], volume);
        self.submit_period(&tmp)
    }
    pub fn fill_sine(
        &mut self,
        phase: &mut u32,
        freq_hz: u32,
        volume: u8,
    ) -> Result<bool, Es1371Error> {
        let mut tmp = [0u8; PERIOD_BYTES];
        let (next, _) =
            generate_sine_s16le_stereo(&mut tmp, *phase, freq_hz, NATIVE_RATE_HZ, volume);
        let submitted = self.submit_period(&tmp)?;
        if submitted {
            *phase = next;
        }
        Ok(submitted)
    }
    pub fn poll_dma_progress_report(&mut self) -> DmaProgress {
        if !self.running {
            return DmaProgress {
                total_frames: self.frames_played,
                first_completed_period: self.last_hw_period as u8,
                completed_periods: 0,
                current_period: self.last_hw_period as u8,
                current_period_frames: 0,
            };
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
        let position = match poll_playback2(
            self.serial,
            |offset| self.read32(offset),
            |offset, value| self.write32(offset, value),
        ) {
            Ok((position, acknowledged)) => {
                self.acknowledgements += u64::from(acknowledged);
                position
            }
            Err(_) => {
                // A failed restore can leave period interrupts disabled. Do
                // not report healthy playback or manufacture cursor progress.
                self.io_failed = true;
                self.stop();
                self.last_position
            }
        };
        let current = position % RING_BYTES as u32;
        let previous = self.last_position % RING_BYTES as u32;
        let advanced = if current >= previous {
            current - previous
        } else {
            RING_BYTES as u32 - previous + current
        };
        let frames = advanced as u64 / FRAME_BYTES as u64;
        self.frames_played = self.frames_played.saturating_add(frames);
        self.last_position = current;
        let current_period = current as usize / PERIOD_BYTES;
        let completed = if current_period >= self.last_hw_period {
            current_period - self.last_hw_period
        } else {
            PERIODS - self.last_hw_period + current_period
        };
        let retired = completed.min(self.filled_periods);
        self.filled_periods -= retired;
        if completed > retired && self.running {
            self.underruns = self.underruns.saturating_add(1);
        }
        let first = self.last_hw_period;
        self.last_hw_period = current_period;
        DmaProgress {
            total_frames: self.frames_played,
            first_completed_period: first as u8,
            completed_periods: retired as u8,
            current_period: current_period as u8,
            current_period_frames: ((current % PERIOD_BYTES as u32) / FRAME_BYTES as u32) as u16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::{Cell, RefCell};
    use std::vec::Vec;

    struct PeriodDevice {
        position: Cell<u32>,
        status: Cell<u32>,
        serial: Cell<u32>,
        writes: RefCell<Vec<(u16, u32)>>,
    }

    impl PeriodDevice {
        fn new() -> Self {
            Self {
                position: Cell::new(0),
                status: Cell::new(0),
                serial: Cell::new(SCTRL_P2_INT_EN | SCTRL_P2_END_INC | SCTRL_P2_MODE_S16_STEREO),
                writes: RefCell::new(Vec::new()),
            }
        }

        // Model a controller that stops at the period boundary until the
        // completion latch is cleared, as observed in the VMware trace.
        fn advance(&self) {
            if self.status.get() & STATUS_DAC2 == 0 {
                self.position
                    .set((self.position.get() + PERIOD_BYTES as u32) % RING_BYTES as u32);
                self.status.set(self.status.get() | STATUS_DAC2);
            }
        }

        fn read(&self, offset: u16) -> Result<u32, Es1371Error> {
            match offset {
                REG_STATUS => Ok(self.status.get()),
                REG_DAC2_SIZE => Ok((self.position.get() << 14) | (RING_BYTES as u32 / 4 - 1)),
                _ => panic!("unexpected register read {offset:#x}"),
            }
        }

        fn write(&self, offset: u16, value: u32) -> Result<(), Es1371Error> {
            assert_eq!(
                offset, REG_SERIAL,
                "ack must not reset DMA or the sample count"
            );
            if self.serial.get() & SCTRL_P2_INT_EN != 0 && value & SCTRL_P2_INT_EN == 0 {
                self.status.set(self.status.get() & !STATUS_DAC2);
            }
            self.serial.set(value);
            self.writes.borrow_mut().push((offset, value));
            Ok(())
        }
    }

    #[test]
    fn acknowledges_each_period_and_keeps_dma_advancing_through_wrap() {
        let dev = PeriodDevice::new();
        let serial = dev.serial.get();
        dev.advance();
        dev.advance();
        assert_eq!(
            dev.position.get(),
            PERIOD_BYTES as u32,
            "unacknowledged engine stalls"
        );
        for period in 1..=PERIODS * 3 {
            let (position, acked) =
                poll_playback2(serial, |r| dev.read(r), |r, v| dev.write(r, v)).unwrap();
            assert!(acked);
            assert_eq!(position, ((period % PERIODS) * PERIOD_BYTES) as u32);
            assert_eq!(dev.status.get() & STATUS_DAC2, 0);
            assert_eq!(dev.serial.get(), serial);
            dev.advance();
        }
        assert_eq!(dev.writes.borrow().len(), PERIODS * 3 * 2);
    }

    #[test]
    fn leaves_other_interrupt_sources_and_serial_bits_untouched() {
        let dev = PeriodDevice::new();
        let unrelated_status = (1 << 31) | (1 << 2) | 1;
        let serial = dev.serial.get() | (1 << 8) | (1 << 10);
        dev.serial.set(serial);
        dev.status.set(unrelated_status);
        let (_, acked) = poll_playback2(serial, |r| dev.read(r), |r, v| dev.write(r, v)).unwrap();
        assert!(!acked);
        assert!(dev.writes.borrow().is_empty());
        dev.advance();
        poll_playback2(serial, |r| dev.read(r), |r, v| dev.write(r, v)).unwrap();
        assert_eq!(dev.status.get(), unrelated_status);
        assert_eq!(
            &*dev.writes.borrow(),
            &[
                (REG_SERIAL, serial & !SCTRL_P2_INT_EN),
                (REG_SERIAL, serial)
            ]
        );
    }

    #[test]
    fn io_failure_is_not_reported_as_successful_progress_or_acknowledgement() {
        let dev = PeriodDevice::new();
        dev.advance();
        let serial = dev.serial.get();
        for failed_read in [REG_STATUS, REG_DAC2_SIZE] {
            let result = poll_playback2(
                serial,
                |r| {
                    if r == failed_read {
                        Err(Es1371Error::Io)
                    } else {
                        dev.read(r)
                    }
                },
                |r, v| dev.write(r, v),
            );
            assert_eq!(result, Err(Es1371Error::Io));
            assert!(dev.writes.borrow().is_empty());
        }
        for failed_write in [1, 2] {
            let writes = Cell::new(0);
            let result = poll_playback2(
                serial,
                |r| dev.read(r),
                |_, _| {
                    writes.set(writes.get() + 1);
                    if writes.get() == failed_write {
                        Err(Es1371Error::Io)
                    } else {
                        Ok(())
                    }
                },
            );
            assert_eq!(result, Err(Es1371Error::Io));
            assert_eq!(writes.get(), failed_write);
        }
    }

    #[test]
    fn validates_aligned_io_bar_and_rejects_memory_bar() {
        assert_eq!(validate_io_bar(0x2001, 0xffff_ffc0), Some((0x2000, 0x40)));
        assert_eq!(validate_io_bar(0x2000, 0xffff_ffc0), None);
    }

    #[test]
    fn rejects_zero_oversized_and_wrapping_resources() {
        assert_eq!(validate_io_bar(0x0001, 0xffff), None);
        assert_eq!(validate_io_bar(0xffc1, 0xffff_c000), None);
        assert_eq!(validate_io_bar(0x2001, 0xffff_0000), None);
    }
}
