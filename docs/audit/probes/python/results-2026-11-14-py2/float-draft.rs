// Binary64 has at most 309 integral and 1074 fractional decimal digits.
// 1536 bytes hold a complete fixed expansion or scientific expansion with
// 1100 fractional digits. Further requested digits are streamed as zeros.
struct Decimal { bytes: [u8; 1536], len: usize }
impl core::fmt::Write for Decimal {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if s.len() > self.bytes.len() - self.len { return Err(core::fmt::Error); }
        self.bytes[self.len..self.len+s.len()].copy_from_slice(s.as_bytes());
        self.len += s.len(); Ok(())
    }
}
fn decimal(out: &mut Output<'_>, value: f64, conversion: u8, precision: Option<usize>, width: usize, left: bool, plus: bool, space: bool, zero: bool, alternate: bool) -> Result<(), ()> {
    use core::fmt::Write;
    let upper = conversion.is_ascii_uppercase();
    let kind = conversion.to_ascii_lowercase();
    let negative = value.is_sign_negative();
    let value = value.abs();
    let mut d = Decimal {bytes: [0;1536],len:0};
    let mut extra = 0;
    let mut exponent = None;
    if !value.is_finite() {
        d.write_str(if value.is_nan() {"nan"} else {"inf"}).map_err(|_|out.error(errno::EOVERFLOW))?;
    } else {
        let p = precision.unwrap_or(6);
        let p = if kind == b'g' {p.max(1)} else {p};
        let scientific = kind == b'e' || kind == b'g';
        let digits = if kind == b'g' {p-1} else {p};
        let bounded = digits.min(1100);
        if scientific {write!(&mut d, "{:.*e}", bounded, value)} else {write!(&mut d, "{:.*}", bounded, value)}.map_err(|_|out.error(errno::EOVERFLOW))?;
        if scientific {
            let e = d.bytes[..d.len].iter().position(|&b|b==b'e').ok_or_else(||out.error(errno::EINVAL))?;
            let exp = core::str::from_utf8(&d.bytes[e+1..d.len]).ok().and_then(|s|s.parse::<i32>().ok()).ok_or_else(||out.error(errno::EINVAL))?;
            if kind == b'g' && exp >= -4 && exp < p as i32 {
                d.len = 0;
                let frac = (p as i64 - 1 - exp as i64).max(0) as usize;
                write!(&mut d, "{:.*}", frac.min(1100), value).map_err(|_|out.error(errno::EOVERFLOW))?;
                extra = frac.saturating_sub(1100);
            } else { d.len=e;exponent=Some(exp);extra=digits.saturating_sub(1100); }
        } else { extra = digits.saturating_sub(1100); }
        if kind == b'g' && !alternate {
            extra=0;
            if d.bytes[..d.len].contains(&b'.') {
                while d.len>0 && d.bytes[d.len-1]==b'0' {d.len-=1;}
                if d.len>0 && d.bytes[d.len-1]==b'.' {d.len-=1;}
            }
        } else if alternate && !d.bytes[..d.len].contains(&b'.') {
            d.write_str(".").map_err(|_|out.error(errno::EOVERFLOW))?;
        }
    }
    let mut ebuf=[0u8;16];let mut elen=0;
    if let Some(exp)=exponent {
        ebuf[0]=if upper {b'E'} else {b'e'};ebuf[1]=if exp<0 {b'-'} else {b'+'};
        let mut n=exp.unsigned_abs();let mut rev=[0;10];let mut len=0;
        loop {rev[len]=b'0'+(n%10) as u8;len+=1;n/=10;if n==0 {break;}}
        if len==1 {rev[len]=b'0';len+=1;}
        for i in 0..len {ebuf[2+i]=rev[len-1-i];}elen=2+len;
    }
    if upper {for b in &mut d.bytes[..d.len] {b.make_ascii_uppercase();}}
    let sign=if negative {Some(b'-')} else if plus {Some(b'+')} else if space {Some(b' ')} else {None};
    let len=d.len.checked_add(extra).and_then(|n|n.checked_add(elen+usize::from(sign.is_some()))).ok_or_else(||out.error(errno::EOVERFLOW))?;
    if len>c_int::MAX as usize {out.error(errno::EOVERFLOW);return Err(());}
    let padding=width.saturating_sub(len);let z=zero && !left && value.is_finite();
    if !left && !z {out.pad(b' ',padding)?;}
    if let Some(s)=sign {out.bytes(&[s])?;}
    if z {out.pad(b'0',padding)?;}
    out.bytes(&d.bytes[..d.len])?;out.pad(b'0',extra)?;out.bytes(&ebuf[..elen])?;
    if left {out.pad(b' ',padding)?;}Ok(())
}
