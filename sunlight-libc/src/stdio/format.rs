//! Allocation-free integer/string printf subset. Unsupported conversions fail
//! explicitly, before consuming their arguments. Uses the pinned SysV VaList ABI.
use super::*;

struct Output<'a> {
    state: Option<&'a mut State>,
    destination: *mut u8,
    capacity: usize,
    count: usize,
}
impl Output<'_> {
    fn error(&mut self, code: c_int) {
        if let Some(state) = self.state.as_deref_mut() {
            state.error = true;
        }
        errno::set_errno(code);
    }
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), ()> {
        if bytes.len() > c_int::MAX as usize - self.count {
            self.error(errno::EOVERFLOW);
            return Err(());
        }
        if let Some(state) = self.state.as_deref_mut() {
            if write_bytes(state, bytes) != bytes.len() {
                return Err(());
            }
        } else {
            let n = bytes.len().min(self.capacity.saturating_sub(self.count));
            if n > 0 {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        self.destination.add(self.count),
                        n,
                    );
                }
            }
        }
        self.count += bytes.len();
        Ok(())
    }
    fn pad(&mut self, byte: u8, mut n: usize) -> Result<(), ()> {
        if n > c_int::MAX as usize - self.count {
            self.error(errno::EOVERFLOW);
            return Err(());
        }
        if self.state.is_none() {
            let stored = n.min(self.capacity.saturating_sub(self.count));
            if stored > 0 {
                unsafe { core::ptr::write_bytes(self.destination.add(self.count), byte, stored) };
            }
            self.count += n;
            return Ok(());
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
            number(fmt, &mut i).ok_or_else(|| out.error(errno::EOVERFLOW))?
        };
        if width > c_int::MAX as usize {
            out.error(errno::EOVERFLOW);
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
                Some(number(fmt, &mut i).ok_or_else(|| out.error(errno::EOVERFLOW))?)
            };
        }
        let length = match fmt.get(i) {
            Some(b'L') => {
                i += 1;
                4
            }
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
            out.error(errno::EINVAL);
            return Err(());
        };
        i += 1;
        if matches!(conversion, b'f' | b'F' | b'e' | b'E' | b'g' | b'G') {
            if length != 0 && length != 3 {
                out.error(errno::ENOSYS);
                return Err(());
            }
            decimal(
                out,
                args.next_arg::<f64>(),
                conversion,
                precision,
                width,
                left,
                plus,
                space,
                zero,
                alternate,
            )?;
            continue;
        }
        if (conversion == b's' || conversion == b'c') && length == 3 {
            wide(out, &mut args, conversion, precision, width, left)?;
            continue;
        }
        if conversion == b's' || conversion == b'c' {
            if length != 0 {
                out.error(errno::ENOSYS);
                return Err(());
            }
            let byte;
            let bytes = if conversion == b'c' {
                byte = [args.next_arg::<c_int>() as u8];
                &byte[..]
            } else {
                let ptr = args.next_arg::<*const u8>();
                if ptr.is_null() {
                    out.error(errno::EINVAL);
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
            out.error(errno::ENOSYS);
            return Err(());
        }
        if conversion == b'p' && length != 0 {
            out.error(errno::EINVAL);
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
    let mut out = Output {
        state: Some(state),
        destination: core::ptr::null_mut(),
        capacity: 0,
        count: 0,
    };
    match render(&mut out, fmt, args) {
        Ok(()) => out.count as c_int,
        Err(()) => -1,
    }
}

// Capacity excludes the final NUL. usize::MAX selects sprintf's caller-owned
// unbounded destination contract, without a temporary output buffer.
pub(super) unsafe fn memory(
    destination: *mut u8,
    size: usize,
    fmt: &[u8],
    args: VaList<'_>,
) -> c_int {
    let mut out = Output {
        state: None,
        destination,
        capacity: size.saturating_sub(1),
        count: 0,
    };
    let result = render(&mut out, fmt, args);
    if size > 0 {
        destination.add(out.count.min(size - 1)).write(0);
    }
    match result {
        Ok(()) => out.count as c_int,
        Err(()) => -1,
    }
}
// Binary64 has at most 309 integral and 1074 fractional decimal digits.
// 1536 bytes hold a complete fixed expansion or scientific expansion with
// 1100 fractional digits. Further requested digits are streamed as zeros.
struct Decimal {
    bytes: [u8; 1536],
    len: usize,
}
impl core::fmt::Write for Decimal {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if s.len() > self.bytes.len() - self.len {
            return Err(core::fmt::Error);
        }
        self.bytes[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
        self.len += s.len();
        Ok(())
    }
}
fn decimal(
    out: &mut Output<'_>,
    value: f64,
    conversion: u8,
    precision: Option<usize>,
    width: usize,
    left: bool,
    plus: bool,
    space: bool,
    zero: bool,
    alternate: bool,
) -> Result<(), ()> {
    use core::fmt::Write;
    let upper = conversion.is_ascii_uppercase();
    let kind = conversion.to_ascii_lowercase();
    let negative = value.is_sign_negative();
    let value = value.abs();
    let mut d = Decimal {
        bytes: [0; 1536],
        len: 0,
    };
    let mut extra = 0;
    let mut exponent = None;
    if !value.is_finite() {
        d.write_str(if value.is_nan() { "nan" } else { "inf" })
            .map_err(|_| out.error(errno::EOVERFLOW))?;
    } else {
        let p = precision.unwrap_or(6);
        let p = if kind == b'g' { p.max(1) } else { p };
        let scientific = kind == b'e' || kind == b'g';
        let digits = if kind == b'g' { p - 1 } else { p };
        let bounded = digits.min(1100);
        if scientific {
            write!(&mut d, "{:.*e}", bounded, value)
        } else {
            write!(&mut d, "{:.*}", bounded, value)
        }
        .map_err(|_| out.error(errno::EOVERFLOW))?;
        if scientific {
            let e = d.bytes[..d.len]
                .iter()
                .position(|&b| b == b'e')
                .ok_or_else(|| out.error(errno::EINVAL))?;
            let exp = core::str::from_utf8(&d.bytes[e + 1..d.len])
                .ok()
                .and_then(|s| s.parse::<i32>().ok())
                .ok_or_else(|| out.error(errno::EINVAL))?;
            if kind == b'g' && exp >= -4 && exp < p as i32 {
                d.len = 0;
                let frac = (p as i64 - 1 - exp as i64).max(0) as usize;
                write!(&mut d, "{:.*}", frac.min(1100), value)
                    .map_err(|_| out.error(errno::EOVERFLOW))?;
                extra = frac.saturating_sub(1100);
            } else {
                d.len = e;
                exponent = Some(exp);
                extra = digits.saturating_sub(1100);
            }
        } else {
            extra = digits.saturating_sub(1100);
        }
        if kind == b'g' && !alternate {
            extra = 0;
            if d.bytes[..d.len].contains(&b'.') {
                while d.len > 0 && d.bytes[d.len - 1] == b'0' {
                    d.len -= 1;
                }
                if d.len > 0 && d.bytes[d.len - 1] == b'.' {
                    d.len -= 1;
                }
            }
        } else if alternate && !d.bytes[..d.len].contains(&b'.') {
            d.write_str(".").map_err(|_| out.error(errno::EOVERFLOW))?;
        }
    }
    let mut ebuf = [0u8; 16];
    let mut elen = 0;
    if let Some(exp) = exponent {
        ebuf[0] = if upper { b'E' } else { b'e' };
        ebuf[1] = if exp < 0 { b'-' } else { b'+' };
        let mut n = exp.unsigned_abs();
        let mut rev = [0; 10];
        let mut len = 0;
        loop {
            rev[len] = b'0' + (n % 10) as u8;
            len += 1;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        if len == 1 {
            rev[len] = b'0';
            len += 1;
        }
        for i in 0..len {
            ebuf[2 + i] = rev[len - 1 - i];
        }
        elen = 2 + len;
    }
    if upper {
        for b in &mut d.bytes[..d.len] {
            b.make_ascii_uppercase();
        }
    }
    let sign = if negative {
        Some(b'-')
    } else if plus {
        Some(b'+')
    } else if space {
        Some(b' ')
    } else {
        None
    };
    let len = d
        .len
        .checked_add(extra)
        .and_then(|n| n.checked_add(elen + usize::from(sign.is_some())))
        .ok_or_else(|| out.error(errno::EOVERFLOW))?;
    if len > c_int::MAX as usize {
        out.error(errno::EOVERFLOW);
        return Err(());
    }
    let padding = width.saturating_sub(len);
    let z = zero && !left && value.is_finite();
    if !left && !z {
        out.pad(b' ', padding)?;
    }
    if let Some(s) = sign {
        out.bytes(&[s])?;
    }
    if z {
        out.pad(b'0', padding)?;
    }
    out.bytes(&d.bytes[..d.len])?;
    out.pad(b'0', extra)?;
    out.bytes(&ebuf[..elen])?;
    if left {
        out.pad(b' ', padding)?;
    }
    Ok(())
}

// The existing LC_CTYPE contract is C ASCII or strict Unicode UTF-8.
// Locale mutation requires external serialization, as for mbrtowc.
unsafe fn wide(
    out: &mut Output<'_>,
    args: &mut VaList<'_>,
    conversion: u8,
    precision: Option<usize>,
    width: usize,
    left: bool,
) -> Result<(), ()> {
    let utf8 = crate::locale::ctype_is_utf8();
    let encode = |value: u32, buffer: &mut [u8; 4]| -> Result<usize, ()> {
        if !utf8 && value > 127 {
            return Err(());
        }
        let c = char::from_u32(value).ok_or(())?;
        Ok(c.encode_utf8(buffer).len())
    };
    if conversion == b'c' {
        let value = args.next_arg::<u32>();
        let mut buffer = [0; 4];
        let n = encode(value, &mut buffer).map_err(|_| out.error(errno::EILSEQ))?;
        if !left {
            out.pad(b' ', width.saturating_sub(n))?;
        }
        out.bytes(&buffer[..n])?;
        if left {
            out.pad(b' ', width.saturating_sub(n))?;
        }
        return Ok(());
    }
    let source = args.next_arg::<*const i32>();
    if source.is_null() {
        out.error(errno::EINVAL);
        return Err(());
    }
    let limit = precision.unwrap_or(usize::MAX);
    let mut length = 0usize;
    let mut chars = 0;
    while length < limit {
        let value = *source.add(chars);
        if value == 0 {
            break;
        }
        let mut buffer = [0; 4];
        let n = encode(value as u32, &mut buffer).map_err(|_| out.error(errno::EILSEQ))?;
        if n > limit - length {
            break;
        }
        if n > c_int::MAX as usize - length {
            out.error(errno::EOVERFLOW);
            return Err(());
        }
        length += n;
        chars += 1;
    }
    if !left {
        out.pad(b' ', width.saturating_sub(length))?;
    }
    for i in 0..chars {
        let mut buffer = [0; 4];
        let n = encode(*source.add(i) as u32, &mut buffer).map_err(|_| out.error(errno::EILSEQ))?;
        out.bytes(&buffer[..n])?;
    }
    if left {
        out.pad(b' ', width.saturating_sub(length))?;
    }
    Ok(())
}
