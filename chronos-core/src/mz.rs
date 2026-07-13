use alloc::vec::Vec;

pub const MZ_HEADER_MIN_SIZE: usize = 28;
pub const MAX_MZ_RELOCATIONS: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutableFormat {
    Com,
    Mz,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedExecutable {
    Pe,
    Ne,
    Le,
    Lx,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MzError {
    TruncatedHeader,
    InvalidMagic,
    InvalidPageCount,
    LogicalSizeExceedsFile,
    HeaderExceedsFile,
    RelocationTableOutsideHeader,
    TooManyRelocations,
    UnsupportedOverlay { overlay: u16 },
    UnsupportedExtendedFormat(UnsupportedExecutable),
    ImageTooLarge,
    RelocationOutsideImage { index: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MzHeader {
    pub e_magic: u16,
    pub e_cblp: u16,
    pub e_cp: u16,
    pub e_crlc: u16,
    pub e_cparhdr: u16,
    pub e_minalloc: u16,
    pub e_maxalloc: u16,
    pub e_ss: u16,
    pub e_sp: u16,
    pub e_csum: u16,
    pub e_ip: u16,
    pub e_cs: u16,
    pub e_lfarlc: u16,
    pub e_ovno: u16,
    pub e_lfanew: Option<u32>,
    pub logical_size: usize,
    pub header_size: usize,
    pub image_size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MzRelocation {
    pub offset: u16,
    pub segment: u16,
}

pub fn classify_executable(image: &[u8]) -> Result<ExecutableFormat, MzError> {
    if image.len() >= 2 && matches!(&image[..2], b"MZ" | b"ZM") {
        parse_mz(image).map(|_| ExecutableFormat::Mz)
    } else {
        Ok(ExecutableFormat::Com)
    }
}

pub fn parse_mz(image: &[u8]) -> Result<MzHeader, MzError> {
    if image.len() < MZ_HEADER_MIN_SIZE {
        return Err(MzError::TruncatedHeader);
    }
    let magic = read_u16(image, 0)?;
    if magic != 0x5a4d && magic != 0x4d5a {
        return Err(MzError::InvalidMagic);
    }
    let e_cblp = read_u16(image, 2)?;
    let e_cp = read_u16(image, 4)?;
    if e_cp == 0 || (e_cblp != 0 && e_cblp > 512) {
        return Err(MzError::InvalidPageCount);
    }
    let logical_size = if e_cblp == 0 {
        e_cp as usize * 512
    } else {
        (e_cp as usize - 1) * 512 + e_cblp as usize
    };
    if logical_size > image.len() {
        return Err(MzError::LogicalSizeExceedsFile);
    }
    let header_size = read_u16(image, 8)? as usize * 16;
    if header_size < MZ_HEADER_MIN_SIZE || header_size > logical_size {
        return Err(MzError::HeaderExceedsFile);
    }
    let e_crlc = read_u16(image, 6)?;
    if e_crlc as usize > MAX_MZ_RELOCATIONS {
        return Err(MzError::TooManyRelocations);
    }
    let e_lfarlc = read_u16(image, 0x18)? as usize;
    let relocation_size = (e_crlc as usize)
        .checked_mul(4)
        .ok_or(MzError::TooManyRelocations)?;
    if e_lfarlc
        .checked_add(relocation_size)
        .is_none_or(|end| end > header_size)
    {
        return Err(MzError::RelocationTableOutsideHeader);
    }
    let e_ovno = read_u16(image, 0x1a)?;
    if e_ovno != 0 {
        return Err(MzError::UnsupportedOverlay { overlay: e_ovno });
    }
    let e_lfanew = if header_size >= 0x40 && image.len() >= 0x40 {
        let value = read_u32(image, 0x3c)?;
        if value != 0 && (value as usize).saturating_add(2) <= logical_size {
            match image.get(value as usize..value as usize + 4) {
                Some(b"PE\0\0") => {
                    return Err(MzError::UnsupportedExtendedFormat(
                        UnsupportedExecutable::Pe,
                    ))
                }
                Some(bytes) if bytes.starts_with(b"NE") => {
                    return Err(MzError::UnsupportedExtendedFormat(
                        UnsupportedExecutable::Ne,
                    ))
                }
                Some(bytes) if bytes.starts_with(b"LE") => {
                    return Err(MzError::UnsupportedExtendedFormat(
                        UnsupportedExecutable::Le,
                    ))
                }
                Some(bytes) if bytes.starts_with(b"LX") => {
                    return Err(MzError::UnsupportedExtendedFormat(
                        UnsupportedExecutable::Lx,
                    ))
                }
                _ => {}
            }
        }
        Some(value)
    } else {
        None
    };
    Ok(MzHeader {
        e_magic: magic,
        e_cblp,
        e_cp,
        e_crlc,
        e_cparhdr: (header_size / 16) as u16,
        e_minalloc: read_u16(image, 0x0a)?,
        e_maxalloc: read_u16(image, 0x0c)?,
        e_ss: read_u16(image, 0x0e)?,
        e_sp: read_u16(image, 0x10)?,
        e_csum: read_u16(image, 0x12)?,
        e_ip: read_u16(image, 0x14)?,
        e_cs: read_u16(image, 0x16)?,
        e_lfarlc: e_lfarlc as u16,
        e_ovno,
        e_lfanew,
        logical_size,
        header_size,
        image_size: logical_size - header_size,
    })
}

pub fn relocations(image: &[u8], header: MzHeader) -> Result<Vec<MzRelocation>, MzError> {
    let mut entries = Vec::with_capacity(header.e_crlc as usize);
    for index in 0..header.e_crlc {
        let base = header.e_lfarlc as usize + index as usize * 4;
        let offset = read_u16(image, base)?;
        let segment = read_u16(image, base + 2)?;
        let linear = segment as usize * 16 + offset as usize;
        if linear
            .checked_add(2)
            .is_none_or(|end| end > header.image_size)
        {
            return Err(MzError::RelocationOutsideImage { index });
        }
        entries.push(MzRelocation { offset, segment });
    }
    Ok(entries)
}

fn read_u16(source: &[u8], offset: usize) -> Result<u16, MzError> {
    let bytes = source
        .get(offset..offset + 2)
        .ok_or(MzError::TruncatedHeader)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(source: &[u8], offset: usize) -> Result<u32, MzError> {
    let bytes = source
        .get(offset..offset + 4)
        .ok_or(MzError::TruncatedHeader)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use super::{
        classify_executable, parse_mz, relocations, ExecutableFormat, MzError,
        UnsupportedExecutable,
    };

    fn image(cblp: u16, cp: u16) -> [u8; 512] {
        let mut image = [0u8; 512];
        image[..2].copy_from_slice(b"MZ");
        image[2..4].copy_from_slice(&cblp.to_le_bytes());
        image[4..6].copy_from_slice(&cp.to_le_bytes());
        image[8..10].copy_from_slice(&2u16.to_le_bytes());
        image[0x18..0x1a].copy_from_slice(&0x1cu16.to_le_bytes());
        image
    }

    #[test]
    fn parses_valid_header_and_last_page_sizes() {
        let first_image = image(64, 1);
        let header = parse_mz(&first_image).unwrap();
        assert_eq!(header.logical_size, 64);
        assert_eq!(header.image_size, 32);
        let zero_last_page = image(0, 1);
        assert_eq!(parse_mz(&zero_last_page).unwrap().logical_size, 512);
    }

    #[test]
    fn rejects_malformed_and_extended_formats() {
        assert_eq!(parse_mz(b"MZ"), Err(MzError::TruncatedHeader));
        let mut image = image(64, 0);
        assert_eq!(parse_mz(&image), Err(MzError::InvalidPageCount));
        image[4..6].copy_from_slice(&1u16.to_le_bytes());
        image[8..10].copy_from_slice(&4u16.to_le_bytes());
        image[0x3c..0x40].copy_from_slice(&0x20u32.to_le_bytes());
        for (signature, kind) in [
            (&b"PE\0\0"[..], UnsupportedExecutable::Pe),
            (&b"NE\0\0"[..], UnsupportedExecutable::Ne),
            (&b"LE\0\0"[..], UnsupportedExecutable::Le),
            (&b"LX\0\0"[..], UnsupportedExecutable::Lx),
        ] {
            image[0x20..0x24].copy_from_slice(signature);
            assert_eq!(
                parse_mz(&image),
                Err(MzError::UnsupportedExtendedFormat(kind))
            );
        }
    }

    #[test]
    fn validates_relocations_and_classifies_com() {
        assert_eq!(classify_executable(&[0x90]).unwrap(), ExecutableFormat::Com);
        let mut image = image(64, 1);
        image[6..8].copy_from_slice(&1u16.to_le_bytes());
        image[0x1c..0x1e].copy_from_slice(&0x0010u16.to_le_bytes());
        image[0x1e..0x20].copy_from_slice(&0u16.to_le_bytes());
        let header = parse_mz(&image).unwrap();
        assert_eq!(relocations(&image, header).unwrap().len(), 1);
        image[0x1c..0x20].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        assert!(matches!(
            relocations(&image, header),
            Err(MzError::RelocationOutsideImage { .. })
        ));
    }
}
