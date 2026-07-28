//! Bounded POSIX-oriented `find` for the native libc utility.
//!
//! Baseline: `find [path...] [expression]`
//!
//! Supported primaries / options:
//! - `-name pattern` — basename shell glob (`*`, `?`)
//! - `-type f|d` — regular file or directory
//! - `-print` — write path + newline (default action)
//! - `-maxdepth N` — limit descent depth (N >= 0)
//!
//! Paths given before the first expression start are search roots.
//! With no roots, `.` is used. Predicates are AND-combined.
//! Depth 0 is the starting path itself.

use sunlight_libc::{DirEntry, Errno, FT_DIR, FT_FILE, MAX_PATH, STDERR, STDOUT};

const MAX_DIR_ENTRIES: usize = 64;
const MAX_STACK: usize = 48;
const MAX_ROOTS: usize = 8;
const MAX_PREDICATES: usize = 8;

pub trait Io {
    fn read_dir(&mut self, path: &[u8], entries: &mut [DirEntry]) -> Result<usize, Errno>;
    /// Classify a path: Ok(true)=dir, Ok(false)=file, Err=missing/unreadable.
    fn is_dir(&mut self, path: &[u8]) -> Result<bool, Errno>;
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno>;
    fn write_stderr(&mut self, bytes: &[u8]);
}

pub fn user_args<'a>(argv: &'a [&'a [u8]]) -> &'a [&'a [u8]] {
    argv.get(1..).unwrap_or(&[])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileKind {
    File,
    Dir,
}

#[derive(Clone, Copy)]
enum Predicate<'a> {
    Name(&'a [u8]),
    Type(FileKind),
}

struct Options<'a> {
    max_depth: Option<usize>,
    print: bool,
    predicates: [Predicate<'a>; MAX_PREDICATES],
    pred_count: usize,
}

impl<'a> Options<'a> {
    fn new() -> Self {
        Self {
            max_depth: None,
            print: true,
            predicates: [Predicate::Name(b""); MAX_PREDICATES],
            pred_count: 0,
        }
    }

    fn push_pred(&mut self, pred: Predicate<'a>) -> bool {
        if self.pred_count >= MAX_PREDICATES {
            return false;
        }
        self.predicates[self.pred_count] = pred;
        self.pred_count += 1;
        true
    }
}

#[derive(Clone, Copy)]
struct StackEntry {
    path: [u8; MAX_PATH],
    len: usize,
    depth: usize,
    is_dir: bool,
}

impl StackEntry {
    fn path_bytes(&self) -> &[u8] {
        &self.path[..self.len]
    }
}

pub fn run(args: &[&[u8]], io: &mut impl Io) -> i32 {
    let (opts, roots) = match parse_args(args, io) {
        Some(v) => v,
        None => return 2,
    };

    let mut code = 0i32;
    if roots.is_empty() {
        if walk_root(b".", true, &opts, io) != 0 {
            code = 1;
        }
    } else {
        for &root in roots {
            let is_dir = true; // roots are treated as walkable; DirEntry type refined below
            if walk_root(root, is_dir, &opts, io) != 0 {
                code = 1;
            }
        }
    }
    code
}

fn walk_root(root: &[u8], _assume_dir: bool, opts: &Options<'_>, io: &mut impl Io) -> i32 {
    if root.len() >= MAX_PATH {
        io.write_stderr(b"find: path too long\n");
        return 1;
    }

    let mut stack: [StackEntry; MAX_STACK] = core::array::from_fn(|_| StackEntry {
        path: [0; MAX_PATH],
        len: 0,
        depth: 0,
        is_dir: false,
    });
    let mut sp = 0usize;

    let root_is_dir = match io.is_dir(root) {
        Ok(v) => v,
        Err(_) => {
            io.write_stderr(b"find: '");
            io.write_stderr(root);
            io.write_stderr(b"': No such file or directory\n");
            return 1;
        }
    };

    push_path(&mut stack, &mut sp, root, 0, root_is_dir);

    let mut code = 0i32;
    while sp > 0 {
        sp -= 1;
        let entry = stack[sp];
        let path = entry.path_bytes();

        if matches_all(path, entry.is_dir, opts) && opts.print {
            if write_path_line(path, io).is_err() {
                code = 1;
                break;
            }
        }

        if !entry.is_dir {
            continue;
        }
        if let Some(max) = opts.max_depth {
            if entry.depth >= max {
                continue;
            }
        }

        let mut entries = [DirEntry::zeroed(); MAX_DIR_ENTRIES];
        let n = match io.read_dir(path, &mut entries) {
            Ok(n) => n,
            Err(_) => {
                io.write_stderr(b"find: cannot read directory ");
                io.write_stderr(path);
                io.write_stderr(b"\n");
                code = 1;
                continue;
            }
        };

        // Push children in reverse so lexicographic-ish order walks forward.
        for i in (0..n).rev() {
            let child = &entries[i];
            let name = child.name_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            let mut child_path = [0u8; MAX_PATH];
            let Some(child_len) = join_path(path, name, &mut child_path) else {
                io.write_stderr(b"find: path too long\n");
                code = 1;
                continue;
            };
            if sp >= MAX_STACK {
                io.write_stderr(b"find: directory stack full\n");
                code = 1;
                continue;
            }
            stack[sp].path = child_path;
            stack[sp].len = child_len;
            stack[sp].depth = entry.depth + 1;
            stack[sp].is_dir = child.file_type == FT_DIR;
            sp += 1;
        }
    }
    code
}

fn push_path(
    stack: &mut [StackEntry; MAX_STACK],
    sp: &mut usize,
    path: &[u8],
    depth: usize,
    is_dir: bool,
) {
    if *sp >= MAX_STACK || path.len() > MAX_PATH {
        return;
    }
    stack[*sp].path[..path.len()].copy_from_slice(path);
    stack[*sp].len = path.len();
    stack[*sp].depth = depth;
    stack[*sp].is_dir = is_dir;
    *sp += 1;
}

fn join_path(parent: &[u8], name: &[u8], out: &mut [u8; MAX_PATH]) -> Option<usize> {
    if parent.is_empty() || parent == b"." {
        if name.len() > MAX_PATH {
            return None;
        }
        out[..name.len()].copy_from_slice(name);
        return Some(name.len());
    }
    if parent == b"/" {
        if name.len() + 1 > MAX_PATH {
            return None;
        }
        out[0] = b'/';
        out[1..1 + name.len()].copy_from_slice(name);
        return Some(1 + name.len());
    }
    let need_slash = !parent.ends_with(b"/");
    let total = parent.len() + if need_slash { 1 } else { 0 } + name.len();
    if total > MAX_PATH {
        return None;
    }
    out[..parent.len()].copy_from_slice(parent);
    let mut n = parent.len();
    if need_slash {
        out[n] = b'/';
        n += 1;
    }
    out[n..n + name.len()].copy_from_slice(name);
    Some(total)
}

fn matches_all(path: &[u8], is_dir: bool, opts: &Options<'_>) -> bool {
    for i in 0..opts.pred_count {
        match opts.predicates[i] {
            Predicate::Name(pat) => {
                let base = basename(path);
                if !glob_match(pat, base) {
                    return false;
                }
            }
            Predicate::Type(FileKind::File) => {
                if is_dir {
                    return false;
                }
            }
            Predicate::Type(FileKind::Dir) => {
                if !is_dir {
                    return false;
                }
            }
        }
    }
    true
}

fn basename(path: &[u8]) -> &[u8] {
    if path.is_empty() {
        return path;
    }
    // Strip trailing slashes except for root.
    let mut end = path.len();
    while end > 1 && path[end - 1] == b'/' {
        end -= 1;
    }
    let trimmed = &path[..end];
    if trimmed == b"/" {
        return trimmed;
    }
    match trimmed.iter().rposition(|&b| b == b'/') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

/// Shell-style glob: `*` (any sequence), `?` (one byte). Other bytes are literal.
fn glob_match(pattern: &[u8], name: &[u8]) -> bool {
    glob_match_rec(pattern, name)
}

fn glob_match_rec(pattern: &[u8], name: &[u8]) -> bool {
    let mut pi = 0usize;
    let mut ni = 0usize;
    let mut star_p = None;
    let mut star_n = 0usize;
    while ni < name.len() {
        if pi < pattern.len() && (pattern[pi] == b'?' || pattern[pi] == name[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < pattern.len() && pattern[pi] == b'*' {
            star_p = Some(pi);
            star_n = ni;
            pi += 1;
        } else if let Some(sp) = star_p {
            pi = sp + 1;
            star_n += 1;
            ni = star_n;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }
    pi == pattern.len()
}

fn write_path_line(path: &[u8], io: &mut impl Io) -> Result<(), Errno> {
    io.write_stdout(path)?;
    io.write_stdout(b"\n")
}

fn parse_args<'a>(args: &'a [&'a [u8]], io: &mut impl Io) -> Option<(Options<'a>, &'a [&'a [u8]])> {
    let mut opts = Options::new();
    let mut i = 0usize;
    // Collect roots until first expression token (starts with `-` or is `!` / `(`).
    while i < args.len() {
        let a = args[i];
        if is_expression_token(a) {
            break;
        }
        i += 1;
    }
    let roots = &args[..i];
    if roots.len() > MAX_ROOTS {
        io.write_stderr(b"find: too many path operands\n");
        return None;
    }

    while i < args.len() {
        let a = args[i];
        match a {
            b"-print" => {
                opts.print = true;
                i += 1;
            }
            b"-name" => {
                i += 1;
                let pat = args.get(i).copied()?;
                if !opts.push_pred(Predicate::Name(pat)) {
                    io.write_stderr(b"find: too many predicates\n");
                    return None;
                }
                i += 1;
            }
            b"-type" => {
                i += 1;
                let t = args.get(i).copied()?;
                let kind = match t {
                    b"f" => FileKind::File,
                    b"d" => FileKind::Dir,
                    _ => {
                        io.write_stderr(b"find: invalid -type argument\n");
                        return None;
                    }
                };
                if !opts.push_pred(Predicate::Type(kind)) {
                    io.write_stderr(b"find: too many predicates\n");
                    return None;
                }
                i += 1;
            }
            b"-maxdepth" => {
                i += 1;
                let n = args.get(i).copied()?;
                opts.max_depth = Some(parse_usize(n)?);
                i += 1;
            }
            // Implicit AND; ignore for baseline.
            b"-a" | b"-and" => i += 1,
            _ => {
                io.write_stderr(b"find: unknown expression: ");
                io.write_stderr(a);
                io.write_stderr(b"\n");
                return None;
            }
        }
    }
    Some((opts, roots))
}

fn is_expression_token(a: &[u8]) -> bool {
    a.starts_with(b"-") || a == b"!" || a == b"(" || a == b")"
}

fn parse_usize(bytes: &[u8]) -> Option<usize> {
    if bytes.is_empty() {
        return None;
    }
    let mut n = 0usize;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(n)
}

pub struct NativeIo;

impl Io for NativeIo {
    fn read_dir(&mut self, path: &[u8], entries: &mut [DirEntry]) -> Result<usize, Errno> {
        sunlight_libc::read_dir(path, entries)
    }
    fn is_dir(&mut self, path: &[u8]) -> Result<bool, Errno> {
        let st = sunlight_libc::stat(path)?;
        Ok(st.file_type == FT_DIR)
    }
    fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno> {
        sunlight_libc::write_all(STDOUT, bytes)
    }
    fn write_stderr(&mut self, bytes: &[u8]) {
        let _ = sunlight_libc::write_all(STDERR, bytes);
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::collections::BTreeMap;
    use std::vec::Vec;

    struct MockFs {
        /// path -> list of (name, is_dir)
        tree: BTreeMap<Vec<u8>, Vec<(Vec<u8>, bool)>>,
        out: Vec<u8>,
        err: Vec<u8>,
    }

    impl MockFs {
        fn new() -> Self {
            Self {
                tree: BTreeMap::new(),
                out: Vec::new(),
                err: Vec::new(),
            }
        }
        fn add_dir(&mut self, path: &[u8], children: &[(&[u8], bool)]) {
            let list = children.iter().map(|(n, d)| (n.to_vec(), *d)).collect();
            self.tree.insert(path.to_vec(), list);
        }
    }

    impl Io for MockFs {
        fn read_dir(&mut self, path: &[u8], entries: &mut [DirEntry]) -> Result<usize, Errno> {
            let Some(children) = self.tree.get(path) else {
                return Err(Errno::NoEntry);
            };
            let n = children.len().min(entries.len());
            for (i, (name, is_dir)) in children.iter().take(n).enumerate() {
                let mut e = DirEntry::zeroed();
                let len = name.len().min(64);
                e.name[..len].copy_from_slice(&name[..len]);
                e.name_len = len as u8;
                e.file_type = if *is_dir { FT_DIR } else { FT_FILE };
                entries[i] = e;
            }
            Ok(n)
        }
        fn is_dir(&mut self, path: &[u8]) -> Result<bool, Errno> {
            if self.tree.contains_key(path) {
                return Ok(true);
            }
            // File leaves appear only as children of a parent directory.
            for (parent, children) in self.tree.iter() {
                for (name, is_dir) in children {
                    let mut full = parent.clone();
                    if full != b"." && full != b"/" && !full.ends_with(b"/") {
                        full.push(b'/');
                    } else if full == b"." {
                        full.clear();
                    }
                    full.extend_from_slice(name);
                    if full.as_slice() == path
                        || (parent.as_slice() == b"." && name.as_slice() == path)
                    {
                        return Ok(*is_dir);
                    }
                }
            }
            Err(Errno::NoEntry)
        }
        fn write_stdout(&mut self, bytes: &[u8]) -> Result<(), Errno> {
            self.out.extend_from_slice(bytes);
            Ok(())
        }
        fn write_stderr(&mut self, bytes: &[u8]) {
            self.err.extend_from_slice(bytes);
        }
    }

    #[test]
    fn prints_tree_with_name_filter() {
        let mut fs = MockFs::new();
        fs.add_dir(
            b".",
            &[(b"a.txt", false), (b"sub", true), (b"b.log", false)],
        );
        fs.add_dir(b"sub", &[(b"c.txt", false)]);
        assert_eq!(run(&[b"-name", b"*.txt"], &mut fs), 0);
        let text = core::str::from_utf8(&fs.out).unwrap();
        assert!(text.contains("a.txt\n"));
        assert!(text.contains("sub/c.txt\n"));
        assert!(!text.contains("b.log"));
    }

    #[test]
    fn type_d_selects_directories() {
        let mut fs = MockFs::new();
        fs.add_dir(b"/tmp", &[(b"f", false), (b"d", true)]);
        fs.add_dir(b"/tmp/d", &[]);
        assert_eq!(run(&[b"/tmp", b"-type", b"d"], &mut fs), 0);
        let text = core::str::from_utf8(&fs.out).unwrap();
        assert!(text.contains("/tmp\n"));
        assert!(text.contains("/tmp/d\n"));
        assert!(!text.contains("/tmp/f\n"));
    }

    #[test]
    fn maxdepth_zero_is_root_only() {
        let mut fs = MockFs::new();
        fs.add_dir(b"/r", &[(b"x", false)]);
        assert_eq!(run(&[b"/r", b"-maxdepth", b"0"], &mut fs), 0);
        assert_eq!(fs.out, b"/r\n");
    }

    #[test]
    fn glob_question_and_star() {
        assert!(glob_match(b"*.txt", b"a.txt"));
        assert!(glob_match(b"a?c", b"abc"));
        assert!(!glob_match(b"a?c", b"ac"));
        assert!(glob_match(b"*", b"anything"));
        assert!(glob_match(b"pre*", b"prefix"));
    }

    #[test]
    fn basename_strips_dirs() {
        assert_eq!(basename(b"/a/b/c.txt"), b"c.txt");
        assert_eq!(basename(b"c.txt"), b"c.txt");
        assert_eq!(basename(b"/"), b"/");
    }
}
