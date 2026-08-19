#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MediaErrorKind {
    None = 0,
    FileOpen = 1,
    FileRead = 2,
    SourceTooLarge = 3,
    UnsupportedContainer = 4,
    UnsupportedCodec = 5,
    MalformedMedia = 6,
    Decode = 7,
    UnsupportedSampleFormat = 8,
    AudioOutput = 9,
    Seek = 10,
    InvalidState = 11,
    Busy = 12,
    Worker = 13,
}

impl MediaErrorKind {
    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::FileOpen,
            2 => Self::FileRead,
            3 => Self::SourceTooLarge,
            4 => Self::UnsupportedContainer,
            5 => Self::UnsupportedCodec,
            6 => Self::MalformedMedia,
            7 => Self::Decode,
            8 => Self::UnsupportedSampleFormat,
            9 => Self::AudioOutput,
            10 => Self::Seek,
            11 => Self::InvalidState,
            12 => Self::Busy,
            13 => Self::Worker,
            _ => Self::None,
        }
    }

    pub const fn user_message(self) -> &'static str {
        match self {
            Self::None => "",
            Self::FileOpen => "Could not open the media file",
            Self::FileRead => "Could not read the media file",
            Self::SourceTooLarge => "This media file is too large for the current player",
            Self::UnsupportedContainer => "The selected file is not an Ogg stream",
            Self::UnsupportedCodec => "This Ogg stream uses an unsupported audio codec",
            Self::MalformedMedia => "The media file is damaged or malformed",
            Self::Decode => "The audio stream could not be decoded",
            Self::UnsupportedSampleFormat => "This audio format is not supported by Sunlight audio",
            Self::AudioOutput => "Sunlight audio output is unavailable",
            Self::Seek => "Could not seek in this media file",
            Self::InvalidState => "That playback action is not available",
            Self::Busy => "The media player is busy",
            Self::Worker => "The media playback worker could not start",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaError {
    pub kind: MediaErrorKind,
    pub detail: u32,
}

impl MediaError {
    pub const fn new(kind: MediaErrorKind, detail: u32) -> Self {
        Self { kind, detail }
    }
}
