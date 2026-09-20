//! Host diagnostic: decode through Melody Mina's decoder into raw S16LE.
use std::{env, fs, io::Write};
use sunlight_media::decoder::{AudioDecoder, ProbeDecoder};

fn main() {
    let args: Vec<_> = env::args().collect();
    assert_eq!(args.len(), 3, "usage: decode_pcm INPUT OUTPUT.s16le");
    let source = fs::read(&args[1]).expect("read input");
    let mut decoder = ProbeDecoder::open(&source).expect("probe input");
    let info = decoder.stream_info();
    assert!(matches!(info.channels, 1 | 2), "unsupported channel layout");
    let mut output = std::io::BufWriter::new(fs::File::create(&args[2]).expect("create output"));
    let mut samples = [0i16; 2048];
    let mut frames = 0u64;
    loop {
        let chunk = decoder.decode(&mut samples).expect("decode input");
        for sample in &samples[..chunk.frames * info.channels as usize] {
            output.write_all(&sample.to_le_bytes()).expect("write PCM");
        }
        frames += chunk.frames as u64;
        if chunk.end_of_stream {
            break;
        }
        assert_ne!(chunk.frames, 0, "decoder made no progress");
    }
    output.flush().expect("flush PCM");
    println!(
        "rate={} channels={} frames={} duration_ms={:?}",
        info.sample_rate_hz,
        info.channels,
        frames,
        info.duration.map(|duration| duration.as_millis())
    );
}
