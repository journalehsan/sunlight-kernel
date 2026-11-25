use super::{DynamicImage, SegmentPlan, SegmentProt};
use core::ops::Range;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Discovery,
    Dynamic,
    Tag(u64),
    Table,
    Bias,
    Unsupported { info: u64, address: u64 },
    Target(u64),
    Permission(u64),
}

pub(super) fn word(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn range(start: u64, size: u64) -> Option<Range<u64>> {
    Some(start..start.checked_add(size)?)
}

fn overlaps(left: &Range<u64>, right: &Range<u64>) -> bool {
    left.start < right.end && right.start < left.end
}

fn segment(plans: &[SegmentPlan], wanted: Range<u64>) -> Option<&SegmentPlan> {
    let mut found = None;
    for plan in plans {
        if plan.mem_size == 0 {
            continue;
        }
        let mapped = range(plan.vaddr, plan.mem_size)?;
        if overlaps(&mapped, &wanted)
            && (mapped.start > wanted.start
                || mapped.end < wanted.end
                || found.replace(plan).is_some())
        {
            return None;
        }
    }
    found
}

pub(super) fn file_range(plans: &[SegmentPlan], wanted: Range<u64>) -> Option<Range<usize>> {
    let plan = segment(plans, wanted.clone())?;
    if wanted.end.checked_sub(plan.vaddr)? > plan.file_size {
        return None;
    }
    let start = plan
        .file_offset
        .checked_add(wanted.start.checked_sub(plan.vaddr)?)?;
    let end = start.checked_add(wanted.end.checked_sub(wanted.start)?)?;
    Some(usize::try_from(start).ok()?..usize::try_from(end).ok()?)
}

/// Validate the bounded, dependency-free x86-64 fixture format. The writer
/// must own an unpublished image and discard it on any returned error.
pub fn apply(
    image: &DynamicImage<'_>,
    plans: &[SegmentPlan],
    phdr: u64,
    mut write: impl FnMut(u64, u64) -> Result<(), Error>,
) -> Result<(), Error> {
    let header = &image.header;
    let ph_size = u64::from(header.phnum)
        .checked_mul(56)
        .ok_or(Error::Discovery)?;
    let ph_end = header.phoff.checked_add(ph_size).ok_or(Error::Discovery)?;
    let headers = image
        .bytes
        .get(
            usize::try_from(header.phoff).map_err(|_| Error::Discovery)?
                ..usize::try_from(ph_end).map_err(|_| Error::Discovery)?,
        )
        .ok_or(Error::Discovery)?;
    let ph_range = range(phdr, ph_size).ok_or(Error::Bias)?;
    let ph_file = file_range(plans, ph_range.clone()).ok_or(Error::Bias)?;
    if ph_file.start as u64 != header.phoff {
        return Err(Error::Bias);
    }
    let mut dynamic = None;
    let mut bias = None;
    for ph in headers.chunks_exact(56) {
        match u32::from_le_bytes(ph[..4].try_into().unwrap()) {
            6 => {
                if word(ph, 8) != header.phoff
                    || word(ph, 32) != ph_size
                    || bias
                        .replace(phdr.checked_sub(word(ph, 16)).ok_or(Error::Bias)?)
                        .is_some()
                {
                    return Err(Error::Bias);
                }
            }
            2 if dynamic.replace(ph).is_some() => return Err(Error::Discovery),
            7 => return Err(Error::Tag(7)),
            _ => {}
        }
    }
    let Some(dynamic) = dynamic else {
        return if header.elf_type == 2 {
            Ok(())
        } else {
            Err(Error::Discovery)
        };
    };
    let bias = bias
        .filter(|bias| bias % 4096 == 0 && (header.elf_type == 3 || *bias == 0))
        .ok_or(Error::Bias)?;
    let dynamic_size = word(dynamic, 32);
    if dynamic_size == 0
        || dynamic_size % 16 != 0
        || dynamic_size > 4096
        || dynamic_size != word(dynamic, 40)
    {
        return Err(Error::Dynamic);
    }
    let dynamic_range = range(
        bias.checked_add(word(dynamic, 16)).ok_or(Error::Dynamic)?,
        dynamic_size,
    )
    .ok_or(Error::Dynamic)?;
    let file = file_range(plans, dynamic_range.clone()).ok_or(Error::Dynamic)?;
    if file.start as u64 != word(dynamic, 8) || overlaps(&ph_range, &dynamic_range) {
        return Err(Error::Dynamic);
    }
    let table = image.bytes.get(file).ok_or(Error::Dynamic)?;
    const TAGS: [u64; 11] = [4, 5, 6, 7, 8, 9, 10, 11, 0x6ffffef5, 0x6ffffff9, 0x6ffffffb];
    let mut values = [None; TAGS.len()];
    let mut ended = false;
    let mut debug_seen = false;
    for entry in table.chunks_exact(16) {
        let (tag, value) = (word(entry, 0), word(entry, 8));
        if ended || tag == 0 {
            if tag != 0 || value != 0 {
                return Err(Error::Dynamic);
            }
            ended = true;
        } else if tag == 21 && value == 0 && !debug_seen {
            debug_seen = true;
        } else {
            let index = TAGS
                .iter()
                .position(|known| *known == tag)
                .ok_or(Error::Tag(tag))?;
            if values[index].replace(value).is_some() {
                return Err(Error::Dynamic);
            }
        }
    }
    let [hash, strings, symbols, rela, size, entry, str_size, sym_size, gnu_hash, count, flags] =
        values;
    if !ended
        || (flags.is_some() && (flags != Some(0x08000000) || header.elf_type != 3))
        || str_size != Some(1)
        || sym_size != Some(24)
    {
        return Err(Error::Dynamic);
    }
    let mut occupied = [Some(ph_range), Some(dynamic_range), None, None, None, None];
    let mut check = |index: usize, address: Option<u64>, size, error| {
        let start = bias.checked_add(address.ok_or(error)?);
        let wanted = start
            .and_then(|start| range(start, size))
            .ok_or(Error::Table)?;
        if occupied
            .iter()
            .flatten()
            .any(|other| overlaps(other, &wanted))
        {
            return Err(Error::Table);
        }
        let file = file_range(plans, wanted.clone()).ok_or(Error::Table)?;
        occupied[index] = Some(wanted);
        image.bytes.get(file).ok_or(Error::Table)
    };
    let (hash, expected): (_, &[u8]) = match (hash, gnu_hash) {
        (Some(hash), None) => (
            Some(hash),
            &[1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ),
        (None, Some(hash)) => (
            Some(hash),
            &[
                1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        ),
        _ => return Err(Error::Dynamic),
    };
    if check(2, hash, expected.len() as u64, Error::Dynamic)? != expected
        || check(3, strings, 1, Error::Dynamic)? != [0]
        || check(4, symbols, 24, Error::Dynamic)? != [0; 24]
    {
        return Err(Error::Dynamic);
    }
    let relocations = match (rela, size, entry, count) {
        (None, None, None, None) => return Ok(()),
        (Some(address), Some(size), Some(24), Some(count))
            if address % 8 == 0
                && size > 0
                && size % 24 == 0
                && size / 24 == count
                && count <= 64
                && header.elf_type == 3 =>
        {
            check(5, Some(address), size, Error::Table)?
        }
        _ => return Err(Error::Table),
    };
    let mut previous = None;
    for rela in relocations.chunks_exact(24) {
        let offset = word(rela, 0);
        let info = word(rela, 8);
        let address = bias.checked_add(offset).ok_or(Error::Target(offset))?;
        if info != 8 {
            return Err(Error::Unsupported { info, address });
        }
        let target = range(address, 8).ok_or(Error::Target(address))?;
        if address % 8 != 0
            || previous.is_some_and(|end| address < end)
            || occupied
                .iter()
                .flatten()
                .any(|other| overlaps(other, &target))
        {
            return Err(Error::Target(address));
        }
        previous = Some(target.end);
        let plan = segment(plans, target).ok_or(Error::Target(address))?;
        if plan.prot != SegmentProt::ReadWrite {
            return Err(Error::Permission(address));
        }
        let value = bias
            .checked_add_signed(word(rela, 16) as i64)
            .ok_or(Error::Bias)?;
        let pointee = range(value, 1).ok_or(Error::Bias)?;
        segment(plans, pointee).ok_or(Error::Bias)?;
        write(address, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ElfHeader;

    const BASE: u64 = 0x2000_0000;
    const PHDR: u64 = BASE + 64;
    const DYNAMIC: usize = 0x100;
    const RELA: usize = 0x250;
    const TARGET: u64 = 0x400;

    fn put(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn fixture(count: u64) -> [u8; 0x500] {
        let mut bytes = [0; 0x500];
        for (offset, value) in [
            (64, 6),
            (72, 64),
            (80, 64),
            (96, 112),
            (120, 2),
            (128, DYNAMIC as u64),
            (136, DYNAMIC as u64),
            (152, 176),
            (160, 176),
            (0x220, 0x1_0000_0001),
        ] {
            put(&mut bytes, offset, value);
        }
        for (index, (tag, value)) in [
            (4, 0x220),
            (5, 0x248),
            (6, 0x230),
            (7, RELA as u64),
            (8, 24 * count),
            (9, 24),
            (10, 1),
            (11, 24),
            (0x6ffffff9, count),
            (0x6ffffffb, 0x08000000),
            (0, 0),
        ]
        .into_iter()
        .enumerate()
        {
            put(&mut bytes, DYNAMIC + index * 16, tag);
            put(&mut bytes, DYNAMIC + index * 16 + 8, value);
        }
        for index in 0..count as usize {
            put(&mut bytes, RELA + index * 24, TARGET + index as u64 * 8);
            put(&mut bytes, RELA + index * 24 + 8, 8);
            put(&mut bytes, RELA + index * 24 + 16, 0x300);
        }
        bytes
    }

    fn run(
        bytes: &[u8],
        prot: SegmentProt,
        phdr: u64,
        write: impl FnMut(u64, u64) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let image = DynamicImage {
            bytes,
            header: ElfHeader {
                entry: 0x300,
                phoff: 64,
                phentsize: 56,
                phnum: 2,
                elf_type: 3,
                machine: 62,
                osabi: 0,
            },
            interpreter: None,
        };
        let plans = [SegmentPlan {
            vaddr: BASE,
            vaddr_page_start: BASE,
            page_count: 1,
            file_offset: 0,
            file_size: bytes.len() as u64,
            mem_size: bytes.len() as u64,
            prot,
        }];
        apply(&image, &plans, phdr, write)
    }

    #[test]
    fn writes_relative_address() {
        let mut writes = 0;
        assert_eq!(
            run(
                &fixture(1),
                SegmentProt::ReadWrite,
                PHDR,
                |address, value| {
                    assert_eq!((address, value), (BASE + TARGET, BASE + 0x300));
                    writes += 1;
                    Ok(())
                }
            ),
            Ok(())
        );
        assert_eq!(writes, 1);
    }

    #[test]
    fn rejects_invalid_metadata_and_targets_before_write() {
        for (offset, value, prot, phdr, expected) in [
            (
                RELA + 8,
                1,
                SegmentProt::ReadWrite,
                PHDR,
                Error::Unsupported {
                    info: 1,
                    address: BASE + TARGET,
                },
            ),
            (
                RELA,
                0x800,
                SegmentProt::ReadWrite,
                PHDR,
                Error::Target(BASE + 0x800),
            ),
            (
                DYNAMIC + 160,
                21,
                SegmentProt::ReadWrite,
                PHDR,
                Error::Dynamic,
            ),
            (
                DYNAMIC + 56,
                u64::MAX - BASE - 7,
                SegmentProt::ReadWrite,
                PHDR,
                Error::Table,
            ),
            (
                DYNAMIC + 56,
                0x230,
                SegmentProt::ReadWrite,
                PHDR,
                Error::Table,
            ),
            (
                RELA,
                TARGET,
                SegmentProt::Read,
                PHDR,
                Error::Permission(BASE + TARGET),
            ),
            (80, 65, SegmentProt::ReadWrite, PHDR, Error::Bias),
            (DYNAMIC, 1, SegmentProt::ReadWrite, PHDR, Error::Tag(1)),
        ] {
            let mut bytes = fixture(1);
            put(&mut bytes, offset, value);
            assert_eq!(
                run(&bytes, prot, phdr, |_, _| panic!("unexpected write")),
                Err(expected)
            );
        }
    }

    #[test]
    fn later_failure_reports_error_after_first_write() {
        let mut bytes = fixture(2);
        put(&mut bytes, RELA + 32, 1);
        let mut written = None;
        let result = run(&bytes, SegmentProt::ReadWrite, PHDR, |address, value| {
            assert!(written.replace((address, value)).is_none());
            Ok(())
        });
        assert_eq!(written, Some((BASE + TARGET, BASE + 0x300)));
        assert_eq!(
            result,
            Err(Error::Unsupported {
                info: 1,
                address: BASE + TARGET + 8
            })
        );
    }
}
