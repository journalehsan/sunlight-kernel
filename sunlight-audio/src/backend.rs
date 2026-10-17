//! Common playback adapter used by audiod. It keeps the PCM queue and stream
//! lifecycle independent of the selected hardware backend.

use crate::hda::DmaProgress;
use crate::{
    es1371::Es1371Playback, hda::HdaPlayback, AudioCapabilities, AudioDeviceState, AudioError,
};

pub enum AudioPlayback {
    Hda(HdaPlayback),
    Es1371(Es1371Playback),
}

impl AudioPlayback {
    pub fn capabilities(&self) -> AudioCapabilities {
        match self {
            Self::Hda(device) => device.capabilities(),
            Self::Es1371(device) => device.capabilities(),
        }
    }

    pub fn input_available(&self) -> bool {
        matches!(self, Self::Hda(device) if device.input_available())
    }
    pub fn poll_capture(&mut self, completed: impl FnMut(&[u8])) {
        if let Self::Hda(device) = self {
            device.poll_capture(completed);
        }
    }
    pub fn capture_lost_periods(&self) -> u64 {
        match self {
            Self::Hda(d) => d.capture_lost_periods(),
            Self::Es1371(_) => 0,
        }
    }
    pub fn start_capture(&mut self) -> Result<(), AudioError> {
        match self {
            Self::Hda(d) => d.start_capture().map_err(Into::into),
            Self::Es1371(_) => Err(AudioError::UnsupportedFormat),
        }
    }
    pub fn stop_capture(&mut self) {
        if let Self::Hda(d) = self {
            d.stop_capture();
        }
    }
    pub fn state(&self) -> AudioDeviceState {
        match self {
            Self::Hda(d) => d.state(),
            Self::Es1371(d) => d.state(),
        }
    }
    pub fn frames_played(&self) -> u64 {
        match self {
            Self::Hda(d) => d.frames_played(),
            Self::Es1371(d) => d.frames_played(),
        }
    }
    pub fn underruns(&self) -> u32 {
        match self {
            Self::Hda(d) => d.underruns(),
            Self::Es1371(d) => d.underruns(),
        }
    }
    pub fn start(&mut self) -> Result<(), AudioError> {
        match self {
            Self::Hda(d) => d.start().map_err(Into::into),
            Self::Es1371(d) => d.start().map_err(Into::into),
        }
    }
    pub fn stop(&mut self) {
        match self {
            Self::Hda(d) => d.stop(),
            Self::Es1371(d) => d.stop(),
        }
    }
    pub fn flush_with_progress(&mut self) -> Result<DmaProgress, AudioError> {
        match self {
            Self::Hda(d) => d.flush_with_progress().map_err(Into::into),
            Self::Es1371(d) => d.flush_with_progress().map_err(Into::into),
        }
    }
    pub fn can_submit_period(&self) -> bool {
        match self {
            Self::Hda(d) => d.can_submit_period(),
            Self::Es1371(d) => d.can_submit_period(),
        }
    }
    pub fn last_submitted_period(&self) -> usize {
        match self {
            Self::Hda(d) => d.last_submitted_period(),
            Self::Es1371(d) => d.last_submitted_period(),
        }
    }
    pub fn poll_dma_progress_report(&mut self) -> DmaProgress {
        match self {
            Self::Hda(d) => d.poll_dma_progress_report(),
            Self::Es1371(d) => d.poll_dma_progress_report(),
        }
    }
    pub fn fill_silence_ready(&mut self) -> Result<u8, AudioError> {
        match self {
            Self::Hda(d) => d.fill_silence_ready().map_err(Into::into),
            Self::Es1371(d) => d.fill_silence_ready().map_err(Into::into),
        }
    }
    pub fn prime_free_silence(&mut self) {
        match self {
            Self::Hda(d) => d.prime_free_silence(),
            Self::Es1371(d) => d.prime_free_silence(),
        }
    }
    pub fn fill_sine(
        &mut self,
        phase: &mut u32,
        freq_hz: u32,
        volume: u8,
    ) -> Result<bool, AudioError> {
        match self {
            Self::Hda(d) => d.fill_sine(phase, freq_hz, volume).map_err(Into::into),
            Self::Es1371(d) => d.fill_sine(phase, freq_hz, volume).map_err(Into::into),
        }
    }
    pub fn fill_pcm(&mut self, pcm: &[u8], volume: u8) -> Result<bool, AudioError> {
        match self {
            Self::Hda(d) => d.fill_pcm(pcm, volume).map_err(Into::into),
            Self::Es1371(d) => d.fill_pcm(pcm, volume).map_err(Into::into),
        }
    }
    pub fn position_frames(&self) -> u32 {
        match self {
            Self::Hda(d) => d.position_frames(),
            Self::Es1371(d) => d.position_frames(),
        }
    }
    pub fn es1371_dma_registers(&self) -> Option<crate::es1371::Es1371DmaRegisters> {
        match self {
            Self::Hda(_) => None,
            Self::Es1371(d) => Some(d.dma_registers()),
        }
    }
}
