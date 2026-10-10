use super::*;
use alloc::{format, rc::Rc};
use core::cell::Cell;
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn live_storage_does_not_require_disk_sync_but_state_storage_does() {
    for path in [
        "/home/demo/Musics/voice.wav",
        "/root/Musics/voice.wav",
        "/tmp/voice.part",
    ] {
        assert!(volatile_path(path));
    }
    for path in [
        "/state/users/1000/voice.wav",
        "/home-other/voice.wav",
        "/tmpdisk/voice.wav",
    ] {
        assert!(!volatile_path(path));
    }
}
#[derive(Clone)]
struct Fs {
    fail_write: Rc<Cell<bool>>,
    fail_sync: Rc<Cell<bool>>,
    fail_publish: Rc<Cell<bool>>,
    max_write: Rc<Cell<usize>>,
}
struct Handle {
    file: Option<fs::File>,
    fs: Fs,
}
fn io(e: std::io::Error) -> Error {
    Error::Io(e.raw_os_error().unwrap_or(5))
}
impl File for Handle {
    fn read(&mut self, dst: &mut [u8]) -> Result<usize, Error> {
        self.file.as_mut().unwrap().read(dst).map_err(io)
    }
    fn write_all(&mut self, src: &[u8]) -> Result<(), Error> {
        self.fs
            .max_write
            .set(self.fs.max_write.get().max(src.len()));
        if self.fs.fail_write.get() {
            return Err(Error::Io(28));
        }
        self.file.as_mut().unwrap().write_all(src).map_err(io)
    }
    fn seek(&mut self, offset: u64) -> Result<(), Error> {
        self.file
            .as_mut()
            .unwrap()
            .seek(SeekFrom::Start(offset))
            .map(|_| ())
            .map_err(io)
    }
    fn sync(&mut self) -> Result<(), Error> {
        if self.fs.fail_sync.get() {
            Err(Error::Io(5))
        } else {
            self.file.as_mut().unwrap().sync_all().map_err(io)
        }
    }
    fn close(&mut self) -> Result<(), Error> {
        self.file = None;
        Ok(())
    }
}
impl FileSystem for Fs {
    type Handle = Handle;
    fn create(&self, path: &str) -> Result<Handle, Error> {
        fs::OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .open(path)
            .map(|f| Handle {
                file: Some(f),
                fs: self.clone(),
            })
            .map_err(io)
    }
    fn open(&self, path: &str) -> Result<Handle, Error> {
        fs::File::open(path)
            .map(|f| Handle {
                file: Some(f),
                fs: self.clone(),
            })
            .map_err(io)
    }
    fn remove(&self, path: &str) -> Result<(), Error> {
        fs::remove_file(path).map_err(io)
    }
    fn publish(&self, staging: &str, target: &str, replace: bool) -> Result<(), Error> {
        if self.fail_publish.get() {
            return Err(Error::Io(5));
        }
        if !replace && std::path::Path::new(target).exists() {
            return Err(Error::Io(17));
        }
        fs::rename(staging, target).map_err(io)
    }
    fn sync_dir(&self, path: &str) -> Result<(), Error> {
        fs::File::open(path).unwrap().sync_all().map_err(io)
    }
}
struct Fixture {
    directory: PathBuf,
    fs: Fs,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "voice-recorder-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        Self {
            directory,
            fs: Fs {
                fail_write: Rc::new(Cell::new(false)),
                fail_sync: Rc::new(Cell::new(false)),
                fail_publish: Rc::new(Cell::new(false)),
                max_write: Rc::new(Cell::new(0)),
            },
        }
    }
    fn path(&self, name: &str) -> String {
        self.directory.join(name).to_str().unwrap().into()
    }
    fn recording(&self) -> Recording<Fs> {
        Recording::create(
            self.fs.clone(),
            self.path("session.part"),
            Format {
                rate: 48000,
                channels: 2,
                bits: 16,
            },
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn wav_finalization_is_external_pcm_and_native_decoder_compatible() {
    use sunlight_media::decoder::{AudioDecoder, WavPcmDecoder};
    let f = Fixture::new();
    let mut r = f.recording();
    let pcm = [0x00, 0x80, 0xff, 0x7f, 0x01, 0x00, 0xff, 0xff];
    r.append(&pcm).unwrap();
    r.finalize().unwrap();
    let bytes = fs::read(&r.path).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(
        u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize,
        bytes.len() - 8
    );
    assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 8);
    assert_eq!(&bytes[44..], &pcm);
    let mut d = WavPcmDecoder::open(&bytes).unwrap();
    let mut samples = [0i16; 4];
    assert_eq!(d.stream_info().sample_rate_hz, 48000);
    assert_eq!(d.stream_info().channels, 2);
    assert_eq!(d.decode(&mut samples).unwrap().frames, 2);
    assert_eq!(samples, [i16::MIN, i16::MAX, 1, -1]);
    assert!(r.finalize().is_err());
    assert!(r.append(&pcm).is_err());
}
#[test]
fn empty_wav_is_valid_and_cannot_save_as_a_recording() {
    let f = Fixture::new();
    let mut r = f.recording();
    r.finalize().unwrap();
    let bytes = fs::read(&r.path).unwrap();
    assert_eq!(bytes.len(), 44);
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36);
    assert!(r
        .save_as(&f.path("saved.wav"), &f.path("stage"), false)
        .is_err());
}
#[test]
fn interrupted_or_discarded_candidate_is_removed() {
    let f = Fixture::new();
    let path;
    {
        let mut r = f.recording();
        path = r.path.clone();
        r.append(&[7; 4096]).unwrap();
    }
    assert!(!std::path::Path::new(&path).exists());
    {
        let mut r = f.recording();
        r.append(&[8; 4096]).unwrap();
        r.finalize().unwrap();
    }
    assert!(!std::path::Path::new(&path).exists());
}
#[test]
fn failed_creation_does_not_truncate_an_existing_file() {
    let f = Fixture::new();
    fs::write(f.path("session.part"), b"previous").unwrap();
    assert!(Recording::create(
        f.fs.clone(),
        f.path("session.part"),
        Format {
            rate: 48000,
            channels: 2,
            bits: 16
        }
    )
    .is_err());
    assert_eq!(fs::read(f.path("session.part")).unwrap(), b"previous");
}
#[test]
fn disk_full_poisoned_candidate_cannot_be_finalized() {
    let f = Fixture::new();
    let mut r = f.recording();
    r.append(&[0; 4096]).unwrap();
    f.fs.fail_write.set(true);
    assert_eq!(r.append(&[0; 4096]), Err(Error::Io(28)));
    assert_eq!(r.frames, 1024);
    f.fs.fail_write.set(false);
    assert_eq!(r.finalize(), Err(Error::State));
    let path = r.path.clone();
    drop(r);
    assert!(!std::path::Path::new(&path).exists());
}
#[test]
fn header_sync_failure_never_marks_recorded() {
    let f = Fixture::new();
    let mut r = f.recording();
    r.append(&[0; 4096]).unwrap();
    f.fs.fail_sync.set(true);
    assert!(r.finalize().is_err());
    assert!(!r.finalized);
    assert!(r.finalize().is_err());
}
#[test]
fn save_failure_preserves_source_and_existing_destination() {
    let f = Fixture::new();
    let mut r = f.recording();
    r.append(&[6; 4096]).unwrap();
    r.finalize().unwrap();
    let before = fs::read(&r.path).unwrap();
    fs::write(f.path("saved.wav"), b"existing").unwrap();
    f.fs.fail_write.set(true);
    assert!(r
        .save_as(&f.path("saved.wav"), &f.path("stage"), true)
        .is_err());
    assert_eq!(fs::read(&r.path).unwrap(), before);
    assert_eq!(fs::read(f.path("saved.wav")).unwrap(), b"existing");
    assert!(!std::path::Path::new(&f.path("stage")).exists());
    assert!(!r.saved);
    f.fs.fail_write.set(false);
    f.fs.fail_publish.set(true);
    assert!(r
        .save_as(&f.path("saved.wav"), &f.path("stage"), true)
        .is_err());
    assert_eq!(fs::read(f.path("saved.wav")).unwrap(), b"existing");
    assert!(!std::path::Path::new(&f.path("stage")).exists());
}
#[test]
fn save_succeeds_and_new_removes_only_the_temporary_file() {
    let f = Fixture::new();
    let mut r = f.recording();
    r.append(&[9; 4096]).unwrap();
    r.finalize().unwrap();
    let source = fs::read(&r.path).unwrap();
    r.save_as(&f.path("saved.wav"), &f.path("stage"), false)
        .unwrap();
    assert!(r.saved);
    let temp = r.path.clone();
    drop(r);
    assert!(!std::path::Path::new(&temp).exists());
    assert_eq!(fs::read(f.path("saved.wav")).unwrap(), source);
}
#[test]
fn streaming_three_minutes_and_save_use_bounded_writes() {
    let f = Fixture::new();
    let mut r = f.recording();
    let batch = [0u8; 32768];
    for _ in 0..1054 {
        r.append(&batch).unwrap();
    }
    r.finalize().unwrap();
    assert_eq!(r.frames, 1054 * 8192);
    assert_eq!(fs::metadata(&r.path).unwrap().len(), 44 + 1054 * 32768);
    r.save_as(&f.path("long.wav"), &f.path("stage"), false)
        .unwrap();
    assert_eq!(f.fs.max_write.get(), 32768);
}
#[test]
fn wav_length_alignment_overflow_and_negotiated_format() {
    use crate::wav::{header, MAX_DATA_BYTES};
    let stereo = Format {
        rate: 48000,
        channels: 2,
        bits: 16,
    };
    assert!(header(stereo, MAX_DATA_BYTES).is_ok());
    assert!(header(stereo, MAX_DATA_BYTES + 4).is_err());
    assert!(header(stereo, 3).is_err());
    assert!(header(Format { rate: 0, ..stereo }, 0).is_err());
    assert!(header(Format { bits: 24, ..stereo }, 0).is_err());
    let mono = header(
        Format {
            rate: 44100,
            channels: 1,
            bits: 16,
        },
        2,
    )
    .unwrap();
    assert_eq!(u32::from_le_bytes(mono[28..32].try_into().unwrap()), 88200);
    assert_eq!(u16::from_le_bytes(mono[32..34].try_into().unwrap()), 2);
}

#[test]
fn cancelled_save_removes_staging_and_retains_recording() {
    let fixture = Fixture::new();
    let source = fixture.path("candidate.part");
    let target = fixture.path("output.wav");
    let stage = fixture.path("stage.part");
    let mut recording = Recording::create(
        fixture.fs.clone(),
        source.clone(),
        Format {
            rate: 48000,
            channels: 2,
            bits: 16,
        },
    )
    .unwrap();
    recording.append(&[0; 32]).unwrap();
    recording.finalize().unwrap();
    assert_eq!(
        recording.save_as_with_cancel(&target, &stage, false, || true),
        Err(Error::Cancelled)
    );
    assert!(std::path::Path::new(&source).exists());
    assert!(!std::path::Path::new(&stage).exists());
    assert!(!std::path::Path::new(&target).exists());
    assert!(!recording.saved);
}
