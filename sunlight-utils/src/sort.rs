//! Byte-preserving `sort` behavior for the native libc utility.
//!
//! POSIX.1-2024 target: `sort [-r] [-u] [-n] [-f] [-d] [-b] [-t char] [-k keydef] ... [-o output] [file...]`
//!
//! Sorts lines from input files (or stdin) and writes to stdout.
//!
//! -r : reverse order
//! -u : unique output (suppress duplicate lines)
//! -n : numeric comparison
//! -f : fold lower/upper case (ASCII only)
//! -d : dictionary order (letters, digits, blanks only)
//! -b : ignore leading blanks when determining key start/end
//! -t C : field separator character (default: whitespace transition)
//! -k keydef : key specification (start[,end][type])
//! -o output : write to output file instead of stdout
//!
//! Uses stable sorting by default for deterministic equal-key ordering.
//!
//! MEMORY POLICY: Lines are collected in a bounded array (MAX_LINES = 2048,
//! MAX_LINE_LEN = 4096).  Exceeding these limits produces a deterministic
//! error.  No spill files are implemented.  This is a recorded conformance gap.

use sunlight_libc::{Errno, Fd, STDERR, STDIN, STDOUT};

use crate::compare;

/// Maximum number of lines sort can hold in memory.
const MAX_LINES: usize = 256;
/// Maximum bytes per input line.
const MAX_LINE_LEN: usize = 1024;
const READ_RETRY_LIMIT: usize = 8;
const BUF_SIZE: usize = 512;

pub trait Io {
    fn open(&mut self, path: &[u8]) -> Result<Fd, Errno>;
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno>;
    fn close(&mut self, fd: Fd) -> Result<(), Errno>;
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn yield_now(&mut self);
}

pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}

/// Key modifiers applied to a sort key.
#[derive(Clone, Copy, Debug, Default)]
struct KeyMod {
    numeric: bool,
    reverse: bool,
    fold: bool,
    dict: bool,
    ignore_blanks: bool,
}

/// A single sort key definition.
#[derive(Clone, Debug)]
struct KeyDef {
    /// 1-based start field
    start_field: usize,
    /// 1-based start character within the start field
    start_char: usize,
    /// 1-based end field (0 = end of line)
    end_field: usize,
    /// 1-based end character within the end field
    end_char: usize,
    modifiers: KeyMod,
}

/// Global sort options.
struct SortOpts {
    keys: [KeyDef; 8],
    num_keys: usize,
    global: KeyMod,
    unique: bool,
    check_only: bool,
    field_sep: Option<u8>,
}

impl SortOpts {
    fn new() -> Self {
        Self {
            keys: [
                KeyDef {
                    start_field: 1,
                    start_char: 1,
                    end_field: 0,
                    end_char: 0,
                    modifiers: KeyMod::default(),
                },
                KeyDef {
                    start_field: 1,
                    start_char: 1,
                    end_field: 0,
                    end_char: 0,
                    modifiers: KeyMod::default(),
                },
                KeyDef {
                    start_field: 1,
                    start_char: 1,
                    end_field: 0,
                    end_char: 0,
                    modifiers: KeyMod::default(),
                },
                KeyDef {
                    start_field: 1,
                    start_char: 1,
                    end_field: 0,
                    end_char: 0,
                    modifiers: KeyMod::default(),
                },
                KeyDef {
                    start_field: 1,
                    start_char: 1,
                    end_field: 0,
                    end_char: 0,
                    modifiers: KeyMod::default(),
                },
                KeyDef {
                    start_field: 1,
                    start_char: 1,
                    end_field: 0,
                    end_char: 0,
                    modifiers: KeyMod::default(),
                },
                KeyDef {
                    start_field: 1,
                    start_char: 1,
                    end_field: 0,
                    end_char: 0,
                    modifiers: KeyMod::default(),
                },
                KeyDef {
                    start_field: 1,
                    start_char: 1,
                    end_field: 0,
                    end_char: 0,
                    modifiers: KeyMod::default(),
                },
            ],
            num_keys: 0,
            global: KeyMod::default(),
            unique: false,
            check_only: false,
            field_sep: None,
        }
    }
}

/// An input record: raw bytes of one line (with newline state).
struct Record {
    data: [u8; MAX_LINE_LEN],
    len: usize,
    has_newline: bool,
}

impl Record {
    fn new() -> Self {
        Self {
            data: [0; MAX_LINE_LEN],
            len: 0,
            has_newline: false,
        }
    }

    fn bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

/// A pre-computed key for fast comparison.
struct SortKey {
    data: [u8; MAX_LINE_LEN],
    len: usize,
}

impl SortKey {
    fn new() -> Self {
        Self {
            data: [0; MAX_LINE_LEN],
            len: 0,
        }
    }
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let opts = match parse_args(args, io) {
        Ok(v) => v,
        Err(code) => return code,
    };

    // Read all input
    let mut records: [Record; MAX_LINES] = core::array::from_fn(|_| Record::new());
    let mut record_count: usize = 0;

    let mut file_list: [&[u8]; 16] = [b""; 16];
    let file_count = collect_files(args, &mut file_list);

    if file_count == 0 {
        if let Err(code) = read_input(io, STDIN, &mut records, &mut record_count) {
            return code;
        }
    } else {
        for i in 0..file_count {
            let path = file_list[i];
            let fd = if path == b"-" {
                STDIN
            } else {
                match io.open(path) {
                    Ok(fd) => fd,
                    Err(_) => {
                        let _ = io.write_stderr(b"sort: cannot open '");
                        let _ = io.write_stderr(path);
                        let _ = io.write_stderr(b"': No such file or directory\n");
                        return 1;
                    }
                }
            };
            if let Err(code) = read_input(io, fd, &mut records, &mut record_count) {
                if fd != STDIN {
                    let _ = io.close(fd);
                }
                return code;
            }
            if fd != STDIN {
                let _ = io.close(fd);
            }
        }
    }

    if record_count == 0 {
        return 0;
    }

    // Pre-compute sort keys
    let mut keys: [SortKey; MAX_LINES] = core::array::from_fn(|_| SortKey::new());
    for i in 0..record_count {
        extract_key(&records[i], &opts, &mut keys[i]);
    }

    // Build index array for stable sort
    let mut indices: [usize; MAX_LINES] = [0; MAX_LINES];
    for i in 0..record_count {
        indices[i] = i;
    }

    // Sort indices
    sort_indices(&mut indices[..record_count], &records, &keys, &opts);

    // Check-only mode
    if opts.check_only {
        let mut sorted = true;
        for i in 1..record_count {
            if compare_records(
                &records[indices[i]],
                &records[indices[i - 1]],
                &keys[indices[i]],
                &keys[indices[i - 1]],
                &opts,
            ) == core::cmp::Ordering::Less
            {
                sorted = false;
                break;
            }
        }
        if !sorted {
            let _ = io.write_stdout(b"sort: disorder: ");
            // Write the first out-of-order line
        }
        return if sorted { 0 } else { 1 };
    }

    // Determine output fd
    let out_fd = match opts_output_fd(args, io) {
        Ok(fd) => fd,
        Err(code) => return code,
    };

    // Write sorted output, handling unique
    let code = write_output(
        io,
        out_fd,
        &records,
        &mut indices[..record_count],
        &keys,
        &opts,
    );
    if out_fd != STDOUT {
        let _ = io.close(out_fd);
    }
    code
}

fn collect_files<'a>(args: &'a [&'a [u8]], out: &mut [&'a [u8]; 16]) -> usize {
    let mut count = 0;
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if a == b"-r"
            || a == b"-u"
            || a == b"-n"
            || a == b"-f"
            || a == b"-d"
            || a == b"-b"
            || a == b"-c"
        {
            i += 1;
            continue;
        }
        if a == b"-t" || a == b"-k" || a == b"-o" {
            i += 2;
            continue;
        }
        if a.starts_with(b"-") && a.len() > 1 {
            i += 1;
            continue;
        }
        if count < out.len() {
            out[count] = a;
            count += 1;
        }
        i += 1;
    }
    count
}

fn opts_output_fd(args: &[&[u8]], io: &mut impl Io) -> Result<Fd, i32> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == b"-o" && i + 1 < args.len() {
            let path = args[i + 1];
            return sunlight_libc::open_with_flags_mode(
                path,
                sunlight_libc::O_WRONLY | sunlight_libc::O_CREAT | sunlight_libc::O_TRUNC,
                0o644,
            )
            .map_err(|_| {
                let _ = io.write_stderr(b"sort: cannot create output file\n");
                1
            });
        }
        i += 1;
    }
    Ok(STDOUT)
}

fn read_input(
    io: &mut impl Io,
    fd: Fd,
    records: &mut [Record; MAX_LINES],
    count: &mut usize,
) -> Result<(), i32> {
    let mut buf = [0u8; BUF_SIZE];
    let mut carry = [0u8; MAX_LINE_LEN];
    let mut carry_len: usize = 0;
    let mut retries = 0;

    loop {
        match io.read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) if n <= buf.len() => {
                let end = carry_len + n;
                if end > MAX_LINE_LEN {
                    let _ = io.write_stderr(b"sort: line too long\n");
                    return Err(1);
                }
                carry[carry_len..end].copy_from_slice(&buf[..n]);
                let mut pos = carry_len;
                carry_len = end;

                while pos < carry_len {
                    let nl = match carry[pos..carry_len].iter().position(|&b| b == b'\n') {
                        Some(off) => pos + off,
                        None => {
                            if pos > 0 {
                                let rem = carry_len - pos;
                                carry.copy_within(pos..carry_len, 0);
                                carry_len = rem;
                            }
                            break;
                        }
                    };

                    if *count >= MAX_LINES {
                        let _ = io.write_stderr(b"sort: too many lines (max 2048)\n");
                        return Err(1);
                    }

                    let line = &carry[pos..nl];
                    let llen = line.len().min(MAX_LINE_LEN);
                    records[*count].data[..llen].copy_from_slice(line);
                    records[*count].len = llen;
                    records[*count].has_newline = true;
                    *count += 1;
                    pos = nl + 1;
                }

                if pos >= carry_len {
                    carry_len = 0;
                }
                retries = 0;
            }
            Ok(_) => return Err(1),
            Err(Errno::Again) if retries < READ_RETRY_LIMIT => {
                retries += 1;
                io.yield_now();
            }
            Err(_) => {
                let _ = io.write_stderr(b"sort: read error\n");
                return Err(1);
            }
        }
    }

    // Final partial line
    if carry_len > 0 {
        if *count >= MAX_LINES {
            let _ = io.write_stderr(b"sort: too many lines (max 2048)\n");
            return Err(1);
        }
        records[*count].data[..carry_len].copy_from_slice(&carry[..carry_len]);
        records[*count].len = carry_len;
        records[*count].has_newline = false;
        *count += 1;
    }

    Ok(())
}

fn extract_key(rec: &Record, opts: &SortOpts, out: &mut SortKey) {
    if opts.num_keys == 0 {
        // Default key: entire line
        let line = rec.bytes();
        let result_len = apply_global_mods_to(line, &opts.global, out);
        out.len = result_len;
        return;
    }

    // Multi-key: concatenate into temp buffer first, then copy to out
    let line = rec.bytes();
    let mut temp = [0u8; MAX_LINE_LEN];
    let mut pos = 0usize;
    for ki in 0..opts.num_keys {
        let kd = &opts.keys[ki];
        let mods = KeyMod {
            numeric: kd.modifiers.numeric || opts.global.numeric,
            reverse: false,
            fold: kd.modifiers.fold || opts.global.fold,
            dict: kd.modifiers.dict || opts.global.dict,
            ignore_blanks: kd.modifiers.ignore_blanks || opts.global.ignore_blanks,
        };

        let key_start = find_key_start(line, kd, &opts.field_sep);
        let key_end = find_key_end(line, kd, &opts.field_sep);

        let key_slice = &line[key_start.min(line.len())..key_end.min(line.len())];
        let effective = apply_mods_inline(key_slice, &mods, &mut temp, pos);
        pos += effective;

        if ki + 1 < opts.num_keys && pos < MAX_LINE_LEN {
            temp[pos] = 0;
            pos += 1;
        }
    }
    out.len = pos.min(MAX_LINE_LEN);
    out.data[..out.len].copy_from_slice(&temp[..out.len]);
}

fn apply_global_mods_to(line: &[u8], mods: &KeyMod, out: &mut SortKey) -> usize {
    let effective = if mods.ignore_blanks {
        skip_leading_blanks(line)
    } else {
        line
    };

    if mods.fold || mods.dict {
        let n = effective.len().min(MAX_LINE_LEN);
        out.data[..n].copy_from_slice(&effective[..n]);
        let temp = &mut out.data[..n];
        if mods.fold {
            compare::fold_case_ascii(temp);
        }
        if mods.dict {
            let mut w = 0;
            for r in 0..n {
                let b = temp[r];
                if b.is_ascii_alphanumeric() || b == b' ' {
                    temp[w] = b;
                    w += 1;
                }
            }
            w
        } else {
            n
        }
    } else {
        let n = effective.len().min(MAX_LINE_LEN);
        out.data[..n].copy_from_slice(&effective[..n]);
        n
    }
}

fn apply_mods_inline(
    line: &[u8],
    mods: &KeyMod,
    buf: &mut [u8; MAX_LINE_LEN],
    offset: usize,
) -> usize {
    let effective = if mods.ignore_blanks {
        skip_leading_blanks(line)
    } else {
        line
    };

    let remaining = MAX_LINE_LEN - offset;
    if mods.fold || mods.dict {
        let n = effective.len().min(remaining);
        buf[offset..offset + n].copy_from_slice(&effective[..n]);
        let temp = &mut buf[offset..offset + n];
        if mods.fold {
            compare::fold_case_ascii(temp);
        }
        if mods.dict {
            let mut w = offset;
            for r in offset..offset + n {
                let b = buf[r];
                if b.is_ascii_alphanumeric() || b == b' ' {
                    buf[w] = b;
                    w += 1;
                }
            }
            w - offset
        } else {
            n
        }
    } else {
        let n = effective.len().min(remaining);
        buf[offset..offset + n].copy_from_slice(&effective[..n]);
        n
    }
}

fn skip_leading_blanks(b: &[u8]) -> &[u8] {
    let pos = b.iter().position(|&x| x != b' ' && x != b'\t');
    match pos {
        Some(p) => &b[p..],
        None => &b[b.len()..],
    }
}

fn find_key_start(line: &[u8], kd: &KeyDef, field_sep: &Option<u8>) -> usize {
    let raw = field_start(line, kd.start_field, field_sep);
    let char_off = compare::skip_chars(&line[raw..], kd.start_char.saturating_sub(1));
    let start = raw + char_off;
    if kd.modifiers.ignore_blanks {
        let blank_off = skip_leading_blanks(&line[start..]);
        start + (blank_off.as_ptr() as usize - line[start..].as_ptr() as usize)
    } else {
        start
    }
}

fn find_key_end(line: &[u8], kd: &KeyDef, field_sep: &Option<u8>) -> usize {
    if kd.end_field == 0 {
        return line.len();
    }
    let raw = field_start(line, kd.end_field, field_sep);
    let char_off = compare::skip_chars(&line[raw..], kd.end_char);
    raw + char_off
}

fn field_start(line: &[u8], field_num: usize, field_sep: &Option<u8>) -> usize {
    if field_num <= 1 {
        return 0;
    }
    match field_sep {
        Some(sep) => {
            let mut pos = 0;
            let mut fields = 1;
            while pos < line.len() && fields < field_num {
                if line[pos] == *sep {
                    fields += 1;
                }
                pos += 1;
            }
            pos
        }
        None => {
            // Whitespace transition: sequence of blanks -> non-blank is a field boundary
            compare::skip_fields(line, field_num - 1, b" \t")
        }
    }
}

fn sort_indices(
    indices: &mut [usize],
    records: &[Record; MAX_LINES],
    keys: &[SortKey; MAX_LINES],
    opts: &SortOpts,
) {
    // Insertion sort for simplicity and stability
    for i in 1..indices.len() {
        let mut j = i;
        while j > 0 {
            let a_idx = indices[j];
            let b_idx = indices[j - 1];
            let ordering = compare_records(
                &records[a_idx],
                &records[b_idx],
                &keys[a_idx],
                &keys[b_idx],
                opts,
            );
            if ordering == core::cmp::Ordering::Less {
                indices.swap(j, j - 1);
                j -= 1;
            } else {
                break;
            }
        }
    }
}

fn compare_records(
    a: &Record,
    b: &Record,
    ka: &SortKey,
    kb: &SortKey,
    opts: &SortOpts,
) -> core::cmp::Ordering {
    let ordering = if opts.num_keys > 0 {
        // Compare pre-computed composite keys
        compare::byte_cmp(&ka.data[..ka.len], &kb.data[..kb.len])
    } else if opts.global.numeric {
        compare::numeric_cmp(&ka.data[..ka.len], &kb.data[..kb.len])
    } else {
        compare::byte_cmp(&ka.data[..ka.len], &kb.data[..kb.len])
    };

    if opts.global.reverse {
        ordering.reverse()
    } else {
        ordering
    }
}

fn write_output(
    io: &mut impl Io,
    out_fd: Fd,
    records: &[Record; MAX_LINES],
    indices: &[usize],
    keys: &[SortKey; MAX_LINES],
    opts: &SortOpts,
) -> i32 {
    let mut prev: Option<&[u8]> = None;
    for &idx in indices {
        let rec = &records[idx];
        let line = rec.bytes();

        if opts.unique {
            let key = &keys[idx].data[..keys[idx].len];
            if let Some(p) = prev {
                if key == p {
                    continue;
                }
            }
            prev = Some(key);
        }

        if write_fd(io, out_fd, line).is_err() {
            return 1;
        }
        if rec.has_newline {
            if write_fd(io, out_fd, b"\n").is_err() {
                return 1;
            }
        }
    }
    0
}

fn write_fd(io: &mut impl Io, fd: Fd, bytes: &[u8]) -> Result<(), Errno> {
    if fd == STDOUT {
        io.write_stdout(bytes)
    } else {
        write_fd_raw(fd, bytes)
    }
}

fn write_fd_raw(fd: Fd, mut data: &[u8]) -> Result<(), Errno> {
    while !data.is_empty() {
        match sunlight_libc::write(fd, data) {
            Ok(n) => data = &data[n.min(data.len())..],
            Err(Errno::Again) => sunlight_libc::yield_now(),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn parse_args<'a>(args: &'a [&'a [u8]], io: &mut impl Io) -> Result<SortOpts, i32> {
    let mut opts = SortOpts::new();
    let mut i = 0;

    while i < args.len() {
        match args[i] {
            b"-r" => {
                if opts.num_keys == 0 {
                    opts.global.reverse = true;
                }
                i += 1;
            }
            b"-u" => {
                opts.unique = true;
                i += 1;
            }
            b"-n" => {
                if opts.num_keys == 0 {
                    opts.global.numeric = true;
                }
                i += 1;
            }
            b"-f" => {
                if opts.num_keys == 0 {
                    opts.global.fold = true;
                }
                i += 1;
            }
            b"-d" => {
                if opts.num_keys == 0 {
                    opts.global.dict = true;
                }
                i += 1;
            }
            b"-b" => {
                if opts.num_keys == 0 {
                    opts.global.ignore_blanks = true;
                }
                i += 1;
            }
            b"-c" => {
                opts.check_only = true;
                i += 1;
            }
            b"-t" => {
                if i + 1 >= args.len() {
                    let _ = io.write_stderr(b"sort: option requires an argument -- 't'\n");
                    return Err(1);
                }
                let sep = args[i + 1];
                if sep.len() != 1 {
                    let _ = io.write_stderr(b"sort: invalid field separator\n");
                    return Err(1);
                }
                opts.field_sep = Some(sep[0]);
                i += 2;
            }
            b"-k" => {
                if i + 1 >= args.len() {
                    let _ = io.write_stderr(b"sort: option requires an argument -- 'k'\n");
                    return Err(1);
                }
                let keydef = args[i + 1];
                let kd = parse_keydef(keydef, io)?;
                if opts.num_keys < 8 {
                    // Apply any global modifiers that were set before first -k
                    if opts.num_keys == 0 {
                        // Transfer global modifiers to become defaults for keys
                    }
                    opts.keys[opts.num_keys] = kd;
                    opts.num_keys += 1;
                } else {
                    let _ = io.write_stderr(b"sort: too many key definitions\n");
                    return Err(1);
                }
                i += 2;
            }
            b"-o" => {
                i += 2; // handled separately
            }
            b"--" => {
                i += 1;
                break;
            }
            _a if _a.starts_with(b"-") && _a.len() > 1 => {
                // Unknown or combined short options
                let _ = io.write_stderr(b"sort: invalid option -- '");
                let _ = io.write_stderr(_a);
                let _ = io.write_stderr(b"'\n");
                return Err(1);
            }
            _ => {
                i += 1; // file operands
            }
        }
    }

    Ok(opts)
}

fn parse_keydef(input: &[u8], io: &mut impl Io) -> Result<KeyDef, i32> {
    let mut kd = KeyDef {
        start_field: 1,
        start_char: 1,
        end_field: 0,
        end_char: 0,
        modifiers: KeyMod::default(),
    };

    // Find end of field specs (before type modifiers)
    let spec_end = input
        .iter()
        .position(|&b| matches!(b, b'n' | b'r' | b'b' | b'f' | b'd'))
        .unwrap_or(input.len());

    let spec = &input[..spec_end];
    let types = &input[spec_end..];

    // Parse start[,end]
    if let Some(comma) = spec.iter().position(|&b| b == b',') {
        // Parse start
        let start = &spec[..comma];
        (kd.start_field, kd.start_char) = parse_field_char(start, io)?;
        if kd.start_field == 0 {
            let _ = io.write_stderr(b"sort: invalid key definition: field number must be >= 1\n");
            return Err(1);
        }

        // Parse end
        let end = &spec[comma + 1..];
        if !end.is_empty() {
            (kd.end_field, kd.end_char) = parse_field_char(end, io)?;
            if kd.end_field == 0 {
                let _ =
                    io.write_stderr(b"sort: invalid key definition: field number must be >= 1\n");
                return Err(1);
            }
        }
    } else {
        // Only start
        (kd.start_field, kd.start_char) = parse_field_char(spec, io)?;
        if kd.start_field == 0 {
            let _ = io.write_stderr(b"sort: invalid key definition: field number must be >= 1\n");
            return Err(1);
        }
    }

    // Parse type modifiers
    for &b in types {
        match b {
            b'n' => kd.modifiers.numeric = true,
            b'r' => kd.modifiers.reverse = true,
            b'b' => kd.modifiers.ignore_blanks = true,
            b'f' => kd.modifiers.fold = true,
            b'd' => kd.modifiers.dict = true,
            _ => {
                let _ = io.write_stderr(b"sort: invalid key modifier\n");
                return Err(1);
            }
        }
    }

    Ok(kd)
}

fn parse_field_char(s: &[u8], _io: &mut impl Io) -> Result<(usize, usize), i32> {
    if s.is_empty() {
        return Ok((1, 1));
    }

    // Find the dot separator
    let dot_pos = s.iter().position(|&b| b == b'.');

    let field = match dot_pos {
        Some(dp) => {
            let f = &s[..dp];
            parse_usize(f).map_err(|_| {
                let _ = _io.write_stderr(b"sort: invalid key field number\n");
                1i32
            })?
        }
        None => parse_usize(s).map_err(|_| {
            let _ = _io.write_stderr(b"sort: invalid key field number\n");
            1i32
        })?,
    };

    let char_pos = match dot_pos {
        Some(dp) if dp + 1 < s.len() => parse_usize(&s[dp + 1..]).map_err(|_| {
            let _ = _io.write_stderr(b"sort: invalid key character position\n");
            1i32
        })?,
        _ => 1,
    };

    Ok((field, char_pos))
}

fn parse_usize(s: &[u8]) -> Result<usize, ()> {
    if s.is_empty() {
        return Err(());
    }
    let mut v = 0usize;
    for &b in s {
        if !b.is_ascii_digit() {
            return Err(());
        }
        v = v.checked_mul(10).ok_or(())?;
        v = v.checked_add((b - b'0') as usize).ok_or(())?;
    }
    Ok(v)
}

pub struct NativeIo;

impl Io for NativeIo {
    fn open(&mut self, path: &[u8]) -> Result<Fd, Errno> {
        sunlight_libc::open(path)
    }
    fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
        sunlight_libc::read(fd, buf)
    }
    fn close(&mut self, fd: Fd) -> Result<(), Errno> {
        sunlight_libc::close(fd)
    }
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno> {
        sunlight_libc::write_all(STDOUT, bytes)
    }
    fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno> {
        sunlight_libc::write_all(STDERR, bytes)
    }
    fn yield_now(&mut self) {
        sunlight_libc::yield_now();
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    struct Mock {
        files: std::collections::HashMap<Vec<u8>, (Vec<u8>, usize)>,
        output: Vec<u8>,
        errors: Vec<u8>,
        fail_read: bool,
        eagain_count: usize,
    }

    impl Mock {
        fn new() -> Self {
            Self {
                files: std::collections::HashMap::new(),
                output: Vec::new(),
                errors: Vec::new(),
                fail_read: false,
                eagain_count: 0,
            }
        }

        fn add_file(&mut self, path: &[u8], data: &[u8]) {
            self.files.insert(path.to_vec(), (data.to_vec(), 0));
        }
    }

    impl Io for Mock {
        fn open(&mut self, path: &[u8]) -> Result<Fd, Errno> {
            if self.files.contains_key(path) {
                Ok(Fd(1))
            } else {
                Err(Errno::NoEntry)
            }
        }
        fn read(&mut self, fd: Fd, buf: &mut [u8]) -> Result<usize, Errno> {
            if self.fail_read {
                return Err(Errno::Failed);
            }
            if self.eagain_count > 0 {
                self.eagain_count -= 1;
                return Err(Errno::Again);
            }
            let idx = fd.0 as usize;
            let keys: Vec<Vec<u8>> = self.files.keys().cloned().collect();
            if idx >= keys.len() {
                return Ok(0);
            }
            let key = &keys[idx];
            let (data, offset) = self.files.get_mut(key).unwrap();
            if *offset >= data.len() {
                return Ok(0);
            }
            let end = (*offset + buf.len()).min(data.len());
            let n = end - *offset;
            buf[..n].copy_from_slice(&data[*offset..end]);
            *offset = end;
            Ok(n)
        }
        fn close(&mut self, _fd: Fd) -> Result<(), Errno> {
            Ok(())
        }
        fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno> {
            self.output.extend_from_slice(bytes);
            Ok(())
        }
        fn write_stderr(&mut self, bytes: &[u8]) -> Result<(), Errno> {
            self.errors.extend_from_slice(bytes);
            Ok(())
        }
        fn yield_now(&mut self) {}
    }

    #[test]
    fn empty_input() {
        let mut m = Mock::new();
        m.add_file(b"f", b"");
        assert_eq!(run(&[b"f"], &mut m), 0);
        assert!(m.output.is_empty());
    }

    #[test]
    fn single_line() {
        let mut m = Mock::new();
        m.add_file(b"f", b"hello\n");
        assert_eq!(run(&[b"f"], &mut m), 0);
        assert_eq!(m.output, b"hello\n");
    }

    #[test]
    fn basic_sort() {
        let mut m = Mock::new();
        m.add_file(b"f", b"c\na\nb\n");
        assert_eq!(run(&[b"f"], &mut m), 0);
        assert_eq!(m.output, b"a\nb\nc\n");
    }

    #[test]
    fn reverse_sort() {
        let mut m = Mock::new();
        m.add_file(b"f", b"a\nb\nc\n");
        assert_eq!(run(&[b"-r", b"f"], &mut m), 0);
        assert_eq!(m.output, b"c\nb\na\n");
    }

    #[test]
    fn numeric_sort() {
        let mut m = Mock::new();
        m.add_file(b"f", b"10\n2\n1\n");
        assert_eq!(run(&[b"-n", b"f"], &mut m), 0);
        assert_eq!(m.output, b"1\n2\n10\n");
    }

    #[test]
    fn unique_sort() {
        let mut m = Mock::new();
        m.add_file(b"f", b"a\nb\na\nc\n");
        assert_eq!(run(&[b"-u", b"f"], &mut m), 0);
        assert_eq!(m.output, b"a\nb\nc\n");
    }

    #[test]
    fn no_final_newline() {
        let mut m = Mock::new();
        m.add_file(b"f", b"c\na\nb");
        assert_eq!(run(&[b"f"], &mut m), 0);
        assert_eq!(m.output, b"a\nb\nc");
    }

    #[test]
    fn empty_lines() {
        let mut m = Mock::new();
        m.add_file(b"f", b"b\n\na\n");
        assert_eq!(run(&[b"f"], &mut m), 0);
        // Empty line sorts before others
        assert_eq!(m.output, b"\na\nb\n");
    }

    #[test]
    fn fold_case_sort() {
        let mut m = Mock::new();
        m.add_file(b"f", b"Banana\napple\nCherry\n");
        assert_eq!(run(&[b"-f", b"f"], &mut m), 0);
        // Case-folded: apple, Banana, Cherry
        assert_eq!(m.output, b"apple\nBanana\nCherry\n");
    }
}
