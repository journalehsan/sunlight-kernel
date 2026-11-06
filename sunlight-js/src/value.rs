use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(pub(crate) u64, pub(crate) usize, pub(crate) u64);

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Undefined,
    Null,
    Boolean(bool),
    Number(f64),
    /// JavaScript strings are sequences of UTF-16 code units, including lone surrogates.
    String(Vec<u16>),
    Object(Handle),
}
impl Value {
    pub fn string(s: &str) -> Self {
        Self::String(s.encode_utf16().collect())
    }
    pub fn truthy(&self) -> bool {
        match self {
            Self::Undefined | Self::Null => false,
            Self::Boolean(b) => *b,
            Self::Number(n) => *n != 0.0 && !n.is_nan(),
            Self::String(s) => !s.is_empty(),
            Self::Object(_) => true,
        }
    }
    pub fn display(&self) -> String {
        match self {
            Self::Undefined => "undefined".into(),
            Self::Null => "null".into(),
            Self::Boolean(b) => b.to_string(),
            Self::Number(n) if *n == 0.0 => "0".into(),
            Self::Number(n) => number_string(*n),
            Self::String(s) => String::from_utf16_lossy(s),
            Self::Object(_) => "[object]".into(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Syntax,
    Reference,
    Type,
    Range,
    Limit,
    Unsupported,
    Exception,
}
#[derive(Clone, Debug)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub source: String,
    /// One-based Unicode scalar line/column; offset is a UTF-8 byte offset.
    pub line: usize,
    pub column: usize,
    pub offset: usize,
    pub thrown: Option<Value>,
}
pub type JsResult<T> = Result<T, Error>;
impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: String::new(),
            line: 1,
            column: 1,
            offset: 0,
            thrown: None,
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {:?}: {}",
            self.source, self.line, self.column, self.kind, self.message
        )
    }
}
impl std::error::Error for Error {}

// Rust supplies shortest round-tripping decimal digits; ECMAScript chooses
// fixed notation for [1e-6, 1e21) and includes '+' in positive exponents.
fn number_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n.is_sign_negative() {
            "-Infinity"
        } else {
            "Infinity"
        }
        .into();
    }
    let text = n.to_string();
    let (sign, digits) = if let Some(d) = text.strip_prefix('-') {
        ("-", d)
    } else {
        ("", text.as_str())
    };
    if n.abs() >= 1e21 || (n != 0.0 && n.abs() < 1e-6) {
        let decimal = digits.find('.').unwrap_or(digits.len());
        let flat = digits.replace('.', "");
        let first = flat.bytes().position(|b| b != b'0').unwrap_or(0);
        let significant = flat[first..].trim_end_matches('0');
        let exponent = decimal as i64 - first as i64 - 1;
        let mantissa = if significant.len() > 1 {
            format!("{}.{}", &significant[..1], &significant[1..])
        } else {
            significant.into()
        };
        format!(
            "{sign}{mantissa}e{}{exponent}",
            if exponent >= 0 { "+" } else { "" }
        )
    } else {
        text
    }
}
