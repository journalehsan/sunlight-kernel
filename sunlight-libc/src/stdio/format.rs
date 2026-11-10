//! Allocation-free integer/string printf subset. Unsupported conversions fail
//! explicitly, before consuming their arguments. Uses the pinned SysV VaList ABI.
use super::*;

struct Output<'a> {
    state: &'a mut State,
    count: usize,
}
impl Output<'_> {
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), ()> {
        if bytes.len() > c_int::MAX as usize - self.count {
            fail(self.state, Errno::TooBig);
            return Err(());
        }
        if write_bytes(self.state, bytes) != bytes.len() {
            return Err(());
        }
        self.count += bytes.len();
        Ok(())
    }
    fn pad(&mut self, byte: u8, mut n: usize) -> Result<(), ()> {
        if n > c_int::MAX as usize - self.count {
            fail(self.state, Errno::TooBig);
            return Err(());
        }
        let block = [byte; 64];
        while n > 0 {
            let take = n.min(block.len());
            self.bytes(&block[..take])?;
            n -= take;
        }
        Ok(())
    }
}
fn number(fmt: &[u8], index: &mut usize) -> Option<usize> {
    let mut value = 0usize;
    while let Some(ch @ b'0'..=b'9') = fmt.get(*index) {
        value = value.checked_mul(10)?.checked_add((ch - b'0') as usize)?;
        if value > c_int::MAX as usize {
            return None;
        }
        *index += 1;
    }
    Some(value)
}
// 0: int, 1: hh, 2: h, 3: long/ll/j/z/t (all 64-bit on native x86-64).
unsafe fn unsigned(args: &mut VaList<'_>, length: u8) -> u64 {
    match length {
        // Both unsigned char and unsigned short promote to int on x86-64.
        1 => args.next_arg::<c_int>() as u8 as u64,
        2 => args.next_arg::<c_int>() as u16 as u64,
        3 => args.next_arg::<u64>(),
        _ => args.next_arg::<u32>() as u64,
    }
}
unsafe fn signed(args: &mut VaList<'_>, length: u8) -> i64 {
    match length {
        1 => args.next_arg::<i32>() as i8 as i64,
        2 => args.next_arg::<i32>() as i16 as i64,
        3 => args.next_arg::<i64>(),
        _ => args.next_arg::<i32>() as i64,
    }
}
unsafe fn render(out: &mut Output<'_>, fmt: &[u8], mut args: VaList<'_>) -> Result<(), ()> {
    let mut i = 0;
    while i < fmt.len() {
        let start = i;
        while i < fmt.len() && fmt[i] != b'%' {
            i += 1;
        }
        out.bytes(&fmt[start..i])?;
        if i == fmt.len() {
            break;
        }
        i += 1;
        if fmt.get(i) == Some(&b'%') {
            out.bytes(b"%")?;
            i += 1;
            continue;
        }
        let (mut left, mut plus, mut space, mut zero, mut alternate) =
            (false, false, false, false, false);
        loop {
            match fmt.get(i) {
                Some(b'-') => left = true,
                Some(b'+') => plus = true,
                Some(b' ') => space = true,
                Some(b'0') => zero = true,
                Some(b'#') => alternate = true,
                _ => break,
            }
            i += 1;
        }
        let width = if fmt.get(i) == Some(&b'*') {
            i += 1;
            let n = args.next_arg::<i32>();
            if n < 0 {
                left = true;
            }
            n.unsigned_abs() as usize
        } else {
            number(fmt, &mut i).ok_or_else(|| fail(out.state, Errno::TooBig))?
        };
        if width > c_int::MAX as usize {
            fail(out.state, Errno::TooBig);
            return Err(());
        }
        let mut precision = None;
        if fmt.get(i) == Some(&b'.') {
            i += 1;
            precision = if fmt.get(i) == Some(&b'*') {
                i += 1;
                let n = args.next_arg::<i32>();
                if n < 0 {
                    None
                } else {
                    Some(n as usize)
                }
            } else {
                Some(number(fmt, &mut i).ok_or_else(|| fail(out.state, Errno::TooBig))?)
            };
        }
        let length = match fmt.get(i) {
            Some(b'h') => {
                i += 1;
                if fmt.get(i) == Some(&b'h') {
                    i += 1;
                    1
                } else {
                    2
                }
            }
            Some(b'l') => {
                i += 1;
                if fmt.get(i) == Some(&b'l') {
                    i += 1;
                }
                3
            }
            Some(b'j' | b'z' | b't') => {
                i += 1;
                3
            }
            _ => 0,
        };
        let Some(&conversion) = fmt.get(i) else {
            fail(out.state, Errno::Inval);
            return Err(());
        };
        i += 1;
        if conversion == b's' || conversion == b'c' {
            if length != 0 {
                errno::set_errno(errno::ENOSYS);
                out.state.error = true;
                return Err(());
            }
            let byte;
            let bytes = if conversion == b'c' {
                byte = [args.next_arg::<c_int>() as u8];
                &byte[..]
            } else {
                let ptr = args.next_arg::<*const u8>();
                if ptr.is_null() {
                    fail(out.state, Errno::Inval);
                    return Err(());
                }
                // A precision bound must not scan beyond that many bytes.
                let limit = precision.unwrap_or(isize::MAX as usize);
                let mut len = 0;
                while len < limit && *ptr.add(len) != 0 {
                    len += 1;
                }
                core::slice::from_raw_parts(ptr, len)
            };
            let pad = width.saturating_sub(bytes.len());
            if !left {
                out.pad(b' ', pad)?;
            }
            out.bytes(bytes)?;
            if left {
                out.pad(b' ', pad)?;
            }
            continue;
        }
        if !matches!(conversion, b'd' | b'i' | b'u' | b'o' | b'x' | b'X' | b'p') {
            out.state.error = true;
            errno::set_errno(errno::ENOSYS);
            return Err(());
        }
        if conversion == b'p' && length != 0 {
            fail(out.state, Errno::Inval);
            return Err(());
        }
        let is_signed = conversion == b'd' || conversion == b'i';
        let mut negative = false;
        let value = if is_signed {
            let n = signed(&mut args, length);
            negative = n < 0;
            n.unsigned_abs()
        } else if conversion == b'p' {
            args.next_arg::<*const c_void>() as usize as u64
        } else {
            unsigned(&mut args, length)
        };
        let radix = match conversion {
            b'o' => 8,
            b'x' | b'X' | b'p' => 16,
            _ => 10,
        };
        let digits = if conversion == b'X' {
            b"0123456789ABCDEF"
        } else {
            b"0123456789abcdef"
        };
        let mut storage = [0; 64];
        let mut start = storage.len();
        let mut rest = value;
        if value != 0 || precision != Some(0) {
            loop {
                start -= 1;
                storage[start] = digits[(rest % radix) as usize];
                rest /= radix;
                if rest == 0 {
                    break;
                }
            }
        }
        let mut prefix = [0; 2];
        let prefix_len = if is_signed && (negative || plus || space) {
            prefix[0] = if negative {
                b'-'
            } else if plus {
                b'+'
            } else {
                b' '
            };
            1
        } else if conversion == b'p' || (alternate && value != 0 && radix == 16) {
            prefix = [b'0', if conversion == b'X' { b'X' } else { b'x' }];
            2
        } else {
            0
        };
        let mut zeros = precision.unwrap_or(0).saturating_sub(storage.len() - start);
        if alternate
            && radix == 8
            && zeros == 0
            && (start == storage.len() || storage[start] != b'0')
        {
            zeros = 1;
        }
        let padding = width.saturating_sub(prefix_len + zeros + storage.len() - start);
        if zero && !left && precision.is_none() {
            zeros += padding;
        } else if !left {
            out.pad(b' ', padding)?;
        }
        out.bytes(&prefix[..prefix_len])?;
        out.pad(b'0', zeros)?;
        out.bytes(&storage[start..])?;
        if left {
            out.pad(b' ', padding)?;
        }
    }
    Ok(())
}
pub(super) unsafe fn print(state: &mut State, fmt: &[u8], args: VaList<'_>) -> c_int {
    if !state.writable {
        fail(state, Errno::BadFd);
        return -1;
    }
    let mut out = Output { state, count: 0 };
    match render(&mut out, fmt, args) {
        Ok(()) => out.count as c_int,
        Err(()) => -1,
    }
}
