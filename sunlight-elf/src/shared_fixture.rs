use super::{
    relative::{file_range, word},
    DynamicImage, SegmentPlan, SegmentProt,
};

pub struct Tables<'a, 'p> {
    image: &'p DynamicImage<'a>,
    plans: &'p [SegmentPlan],
    bias: u64,
    tags: [Option<u64>; TAGS.len()],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TestFault {
    MissingExport,
    DuplicateExport,
    UndefinedImport,
    SymbolRange,
    RelocationRange,
    RelocationType,
    AlternateExport,
}

const TAGS: [u64; 16] = [1, 2, 3, 4, 5, 6, 10, 11, 14, 20, 21, 23, 0x6ffffffb, 7, 8, 9];

pub fn validate_dependency_name(name: &[u8]) -> Result<(), &'static str> {
    if name.is_empty() || name == b"." || name == b".."
        || !name.iter().all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(byte))
    {
        return Err("dependency path");
    }
    Ok(())
}

impl<'a, 'p> Tables<'a, 'p> {
    pub fn parse(
        image: &'p DynamicImage<'a>,
        plans: &'p [SegmentPlan],
        bias: u64,
        library: bool,
    ) -> Result<Self, &'static str> {
        Self::parse_inner(image, plans, bias, library, false)
    }

    pub fn parse_startup_tls(
        image: &'p DynamicImage<'a>,
        plans: &'p [SegmentPlan],
        bias: u64,
        library: bool,
    ) -> Result<Self, &'static str> {
        Self::parse_inner(image, plans, bias, library, true)
    }

    fn parse_inner(
        image: &'p DynamicImage<'a>,
        plans: &'p [SegmentPlan],
        bias: u64,
        library: bool,
        startup_tls: bool,
    ) -> Result<Self, &'static str> {
        let mut result = Self {
            image,
            plans,
            bias,
            tags: [None; TAGS.len()],
        };
        let stage = "dynamic-table parsing";
        let start = image.header.phoff as usize;
        let end = start + usize::from(image.header.phnum) * 56;
        let mut dynamic = None;
        for header in image.bytes[start..end].chunks_exact(56) {
            match u32::from_le_bytes(header[..4].try_into().unwrap()) {
                2 if dynamic.replace(header).is_some() => return Err(stage),
                7 if !startup_tls => return Err("TLS program header"),
                0x6474e552 => return Err("unsupported RELRO"),
                _ => {}
            }
        }
        let header = dynamic.ok_or(stage)?;
        let size = word(header, 32);
        if size == 0 || size > 4096 || size % 16 != 0 || size != word(header, 40) {
            return Err(stage);
        }
        let file = result.file(word(header, 16), size).ok_or(stage)?;
        if file.start as u64 != word(header, 8) {
            return Err(stage);
        }
        let mut ended = false;
        for entry in image.bytes.get(file).ok_or(stage)?.chunks_exact(16) {
            let (tag, value) = (word(entry, 0), word(entry, 8));
            if tag == 0 && value == 0 {
                ended = true;
                continue;
            }
            let index = TAGS.iter().position(|known| *known == tag).ok_or(stage)?;
            if ended
                || (matches!(tag, 7 | 8 | 9) && !(library && startup_tls))
                || (library && matches!(tag, 21 | 0x6ffffffb))
                || (!library && tag == 14)
                || result.tags[index].replace(value).is_some()
            {
                return Err(stage);
            }
        }
        if !ended
            || (!library
                && (result.tag(21).unwrap_or(0) != 0 || result.tag(0x6ffffffb).unwrap_or(0) & !0x08000000 != 0))
        {
            return Err(stage);
        }
        Ok(result)
    }

    fn tag(&self, tag: u64) -> Option<u64> {
        self.tags[TAGS.iter().position(|known| *known == tag).unwrap()]
    }

    fn file(&self, address: u64, size: u64) -> Option<core::ops::Range<usize>> {
        let start = self.bias.checked_add(address)?;
        file_range(self.plans, start..start.checked_add(size)?)
    }

    fn bytes(&self, tag: u64, size: u64) -> Option<&'a [u8]> {
        self.image.bytes.get(self.file(self.tag(tag)?, size)?)
    }

    fn string(&self, offset: u64) -> Result<&'a [u8], &'static str> {
        let stage = "string-table validation";
        let strings = self.bytes(5, self.tag(10).ok_or(stage)?).ok_or(stage)?;
        if strings.first() != Some(&0) || strings.last() != Some(&0) {
            return Err(stage);
        }
        let tail = strings
            .get(usize::try_from(offset).map_err(|_| stage)?..)
            .ok_or(stage)?;
        Ok(&tail[..tail.iter().position(|byte| *byte == 0).ok_or(stage)?])
    }

    pub fn identity(&self, library: bool, name: &[u8]) -> Result<(), &'static str> {
        let stage = if library {
            "library ELF validation"
        } else {
            "DT_NEEDED validation"
        };
        if self.string(self.tag(if library { 14 } else { 1 }).ok_or(stage)?)? != name {
            return Err(stage);
        }
        Ok(())
    }

    pub fn needed(&self, invalid_offset: bool) -> Result<Option<&'a [u8]>, &'static str> {
        let name = self.tag(1)
            .map(|offset| self.string(if invalid_offset { u64::MAX } else { offset }))
            .transpose()?;
        if let Some(name) = name {
            validate_dependency_name(name)?;
        }
        Ok(name)
    }

    fn symbols(&self) -> Result<&'a [u8], &'static str> {
        let stage = "symbol-table validation";
        let hash = self.bytes(4, 8).ok_or(stage)?;
        let buckets = u32::from_le_bytes(hash[..4].try_into().unwrap());
        let chains = u32::from_le_bytes(hash[4..].try_into().unwrap());
        if !(2..=4096).contains(&chains) || !(1..=4096).contains(&buckets) || self.tag(11) != Some(24) {
            return Err(stage);
        }
        let hash = self
            .bytes(4, 8 + 4 * u64::from(buckets + chains))
            .ok_or(stage)?;
        if hash[8..]
            .chunks_exact(4)
            .any(|entry| u32::from_le_bytes(entry.try_into().unwrap()) >= chains)
        {
            return Err(stage);
        }
        let symbols = self.bytes(6, 24 * u64::from(chains)).ok_or(stage)?;
        if symbols[..24] != [0; 24] {
            return Err(stage);
        }
        Ok(symbols)
    }

    fn name(&self, symbol: &[u8]) -> Result<&'a [u8], &'static str> {
        self.string(u64::from(u32::from_le_bytes(symbol[..4].try_into().unwrap())))
    }

    pub fn import(&self, index: usize, fault: Option<TestFault>) -> Result<&'a [u8], &'static str> {
        self.slot_symbol(index, fault, false)
    }

    pub fn slot_symbol(&self, index: usize, fault: Option<TestFault>, allow_defined: bool) -> Result<&'a [u8], &'static str> {
        let table = self.symbols()?;
        let stage = "undefined application symbol validation";
        let index = if fault == Some(TestFault::UndefinedImport) { 0 } else { index };
        let symbol = table.chunks_exact(24).nth(index).ok_or(stage)?;
        if index == 0 || symbol[4..6] != [0x12, 0]
            || (symbol[6..8] == [0, 0] && symbol[8..] != [0; 16])
            || (!allow_defined && symbol[6..8] != [0, 0])
        {
            return Err(stage);
        }
        let name = self.name(symbol)?;
        if name.is_empty() || !name.iter().all(|byte| byte.is_ascii_graphic()) {
            return Err(stage);
        }
        Ok(name)
    }

    pub fn export(&self, name: &[u8], fault: Option<TestFault>) -> Result<u64, &'static str> {
        self.find_export(name, fault)?.ok_or("missing library export")
    }

    pub fn find_export(&self, name: &[u8], fault: Option<TestFault>) -> Result<Option<u64>, &'static str> {
        let mut definition = None;
        for symbol in self.symbols()?.chunks_exact(24).skip(1) {
            let matches = self.name(symbol)? == name && symbol[6..8] != [0, 0];
            let matches = match fault {
                Some(TestFault::MissingExport) => false,
                Some(TestFault::DuplicateExport) => true,
                Some(TestFault::AlternateExport) => symbol[4..6] == [0x12, 0] && symbol[6..8] != [0, 0],
                _ => matches,
            };
            if !matches {
                continue;
            }
            if definition.replace(symbol).is_some() {
                return Err("duplicate library export");
            }
            if fault == Some(TestFault::AlternateExport) {
                break;
            }
        }
        let Some(symbol) = definition else { return Ok(None) };
        let stage = "library symbol range or type";
        let section = u16::from_le_bytes(symbol[6..8].try_into().unwrap());
        let (value, size) = (word(symbol, 8), word(symbol, 16));
        let section_count = u16::from_le_bytes(self.image.bytes[60..62].try_into().unwrap());
        if symbol[4..6] != [0x12, 0] || section == 0 || section >= 0xff00
            || (section_count != 0 && section >= section_count) || size == 0 {
            return Err(stage);
        }
        let value = if fault == Some(TestFault::SymbolRange) { u64::from(u32::MAX) } else { value };
        let start = self.bias.checked_add(value).ok_or(stage)?;
        let end = start.checked_add(size).ok_or(stage)?;
        if self.file(value, size).is_none()
            || !self.plans.iter().any(|plan| {
                plan.prot == SegmentProt::ReadExec
                    && start >= plan.vaddr
                    && end <= plan.vaddr + plan.mem_size
            })
        {
            return Err(stage);
        }
        Ok(Some(start))
    }

    pub fn jump_slot(&self, fault: Option<TestFault>) -> Result<(u64, usize, i64), &'static str> {
        self.jump_slots::<1>(fault).map(|[slot]| slot)
    }

    pub fn utility_exports(&self, names: &[&[u8]], unexpected_needed: bool) -> Result<(), &'static str> {
        if self.needed(false)?.is_some() || unexpected_needed {
            return Err("utility DT_NEEDED");
        }
        if [2, 3, 20, 23].into_iter().any(|tag| self.tag(tag).is_some()) {
            return Err("utility library relocations");
        }
        if self.symbols()?.len() / 24 != names.len() + 1 {
            return Err("utility export inventory");
        }
        for name in names {
            self.export(name, None)?;
        }
        Ok(())
    }

    pub fn jump_slots<const N: usize>(
        &self,
        fault: Option<TestFault>,
    ) -> Result<[(u64, usize, i64); N], &'static str> {
        let size = 24 * N as u64;
        if self.tag(2) != Some(size) || self.tag(20) != Some(7) {
            return Err("relocation");
        }
        let mut slots = [(0, 0, 0); N];
        let entries = self.bytes(23, size).ok_or("relocation")?;
        for (index, entry) in entries.chunks_exact(24).enumerate() {
            let slot = self.parse_jump_slot(entry, fault)?;
            if slots[..index].iter().any(|previous| previous.0 == slot.0) {
                return Err("duplicate jump slot");
            }
            slots[index] = slot;
        }
        Ok(slots)
    }

    fn parse_jump_slot(&self, entry: &[u8], fault: Option<TestFault>) -> Result<(u64, usize, i64), &'static str> {
        let stage = "relocation";
        let info = if fault == Some(TestFault::RelocationType) { 1 } else { word(entry, 8) };
        if info as u32 != 7 {
            return Err("unsupported relocation");
        }
        let offset = if fault == Some(TestFault::RelocationRange) { u64::from(u32::MAX) & !7 } else { word(entry, 0) };
        let addend = word(entry, 16) as i64;
        if addend != 0 {
            return Err("unsupported JUMP_SLOT addend");
        }
        let address = self.bias.checked_add(offset).ok_or(stage)?;
        let end = address.checked_add(8).ok_or(stage)?;
        if address % 8 != 0 || !self.plans.iter().any(|plan| {
                plan.prot == SegmentProt::ReadWrite
                    && address >= plan.vaddr
                    && end <= plan.vaddr + plan.mem_size
            })
        {
            return Err("GOT/PLT");
        }
        Ok((address, (info >> 32) as usize, addend))
    }

    pub fn startup_tls_relocations(&self, invalid_type: bool) -> Result<(u64, u64), &'static str> {
        let stage = "startup TLS relocation";
        if self.tag(8) != Some(24) || self.tag(9) != Some(24) || self.tag(1).is_some() {
            return Err(stage);
        }
        let entry = self.bytes(7, 24).ok_or(stage)?;
        let info = if invalid_type { 17 } else { word(entry, 8) };
        if info != 16 || word(entry, 16) != 0 {
            return Err("unsupported startup TLS relocation");
        }
        let offset = word(entry, 0);
        let end_offset = offset.checked_add(16).ok_or(stage)?;
        let start = self.bias.checked_add(offset).ok_or(stage)?;
        let end = self.bias.checked_add(end_offset).ok_or(stage)?;
        if start % 8 != 0 || !self.plans.iter().any(|plan| {
            plan.prot == SegmentProt::ReadWrite && start >= plan.vaddr
                && plan.vaddr.checked_add(plan.mem_size).is_some_and(|limit| end <= limit)
        }) {
            return Err(stage);
        }
        let index = self.image.bytes.get(self.file(offset, 16).ok_or(stage)?).ok_or(stage)?;
        if index != [0; 16] {
            return Err("invalid startup TLS index");
        }
        let (slot, symbol_index, _) = self.jump_slot(None)?;
        if (start..end).contains(&slot) || symbol_index == 0 {
            return Err(stage);
        }
        let symbol = self.symbols()?.chunks_exact(24).nth(symbol_index).ok_or(stage)?;
        if symbol[4] != 0x10 || symbol[5..] != [0; 19] || self.name(symbol)? != b"__tls_get_addr" {
            return Err("invalid startup TLS resolver import");
        }
        Ok((start, slot))
    }
}
