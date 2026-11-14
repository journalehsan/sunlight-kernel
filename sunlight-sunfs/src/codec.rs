//! Explicit little-endian revision-2 encoding. No native layout casts.
use crate::{Error, Result};
use alloc::{vec, vec::Vec};
pub const PAGE: usize = 4096;
pub const HEADER: usize = 96;
pub const PAYLOAD: usize = PAGE - HEADER;
pub const REQUIRED_FEATURES: u64 = 3;
pub fn crc64(bytes: &[u8]) -> u64 {
    let mut crc = 0u64;
    for &byte in bytes {
        crc ^= (byte as u64) << 56;
        for _ in 0..8 {
            crc = if crc & (1 << 63) != 0 {
                (crc << 1) ^ 0x42f0e1eba9ea3693
            } else {
                crc << 1
            };
        }
    }
    crc
}
pub struct Encoder {
    bytes: Vec<u8>,
}
impl Encoder {
    pub fn new() -> Self {
        Self { bytes: Vec::new() }
    }
    pub fn put(&mut self, b: &[u8]) -> Result<()> {
        if self
            .bytes
            .len()
            .checked_add(b.len())
            .ok_or(Error::Overflow)?
            > PAYLOAD
        {
            return Err(Error::Bounds);
        }
        self.bytes
            .try_reserve(b.len())
            .map_err(|_| Error::NoMemory)?;
        self.bytes.extend_from_slice(b);
        Ok(())
    }
    pub fn field<T: Field>(&mut self, v: &T) -> Result<()> {
        v.write(self)
    }
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}
impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}
pub struct Decoder<'a> {
    bytes: &'a [u8],
    cursor: usize,
}
impl<'a> Decoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() > PAYLOAD {
            return Err(Error::Bounds);
        }
        Ok(Self { bytes, cursor: 0 })
    }
    pub fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.cursor.checked_add(len).ok_or(Error::Overflow)?;
        let slice = self.bytes.get(self.cursor..end).ok_or(Error::Bounds)?;
        self.cursor = end;
        Ok(slice)
    }
    pub fn field<T: Field>(&mut self) -> Result<T> {
        T::read(self)
    }
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.cursor
    }
    pub fn end(&self) -> Result<()> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(Error::Structure)
        }
    }
}
pub trait Field: Sized {
    fn write(&self, e: &mut Encoder) -> Result<()>;
    fn read(d: &mut Decoder<'_>) -> Result<Self>;
}
macro_rules! integer {
    ($t:ty,$n:expr) => {
        impl Field for $t {
            fn write(&self, e: &mut Encoder) -> Result<()> {
                e.put(&self.to_le_bytes())
            }
            fn read(d: &mut Decoder<'_>) -> Result<Self> {
                Ok(<$t>::from_le_bytes(
                    d.take($n)?.try_into().map_err(|_| Error::Bounds)?,
                ))
            }
        }
    };
}
integer!(u8, 1);
integer!(u16, 2);
integer!(u32, 4);
integer!(u64, 8);
integer!(i64, 8);
impl<const N: usize> Field for [u8; N] {
    fn write(&self, e: &mut Encoder) -> Result<()> {
        e.put(self)
    }
    fn read(d: &mut Decoder<'_>) -> Result<Self> {
        d.take(N)?.try_into().map_err(|_| Error::Bounds)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Zero<const N: usize>;
impl<const N: usize> Default for Zero<N> {
    fn default() -> Self {
        Self
    }
}
impl<const N: usize> Field for Zero<N> {
    fn write(&self, e: &mut Encoder) -> Result<()> {
        e.put(&[0; N])
    }
    fn read(d: &mut Decoder<'_>) -> Result<Self> {
        if d.take(N)?.iter().any(|&v| v != 0) {
            Err(Error::Reserved)
        } else {
            Ok(Self)
        }
    }
}
pub trait Validate {
    fn validate(&self) -> Result<()>;
}
macro_rules! record {
    ($name:ident { $($f:ident:$t:ty),* $(,)? }, $check:expr) => {
        #[derive(Clone,Debug,Default,PartialEq,Eq)]
        pub struct $name { $(pub $f:$t),* }
        impl Validate for $name { fn validate(&self)->Result<()> { ($check)(self) } }
        impl Field for $name {
            fn write(&self,e:&mut Encoder)->Result<()> { self.validate()?; $(e.field(&self.$f)?;)* Ok(()) }
            fn read(d:&mut Decoder<'_>)->Result<Self> { let v=Self { $($f:d.field()?),* }; v.validate()?; Ok(v) }
        }
    }
}
fn need(v: bool) -> Result<()> {
    if v {
        Ok(())
    } else {
        Err(Error::Structure)
    }
}
record!(
    NodeId {
        nid: u64,
        incarnation: u64
    },
    |v: &NodeId| need((v.nid == 0) == (v.incarnation == 0))
);
impl NodeId {
    pub const ROOT: Self = Self {
        nid: 1,
        incarnation: 1,
    };
    pub fn present(&self) -> Result<()> {
        need(self.nid != 0 && self.incarnation != 0)
    }
    pub fn absent(&self) -> bool {
        self.nid == 0 && self.incarnation == 0
    }
}
record!(
    BlockRef {
        block: u64,
        epoch: u64,
        crc: u64
    },
    |v: &BlockRef| need(if v.block == 0 {
        v.epoch == 0 && v.crc == 0
    } else {
        v.epoch != 0
    })
);
impl BlockRef {
    pub fn absent(&self) -> bool {
        self.block == 0 && self.epoch == 0 && self.crc == 0
    }
    pub fn present(&self) -> Result<()> {
        self.validate()?;
        need(!self.absent())
    }
}
record!(DiskTime { seconds:i64,nanos:u32,reserved:Zero<4> }, |v:&DiskTime| need(v.nanos<1_000_000_000));
record!(
    Features {
        compatible: u64,
        readonly: u64,
        incompatible: u64
    },
    |v: &Features| {
        if v.incompatible != REQUIRED_FEATURES {
            Err(Error::Unsupported)
        } else {
            Ok(())
        }
    }
);
record!(Superblock {
    major:u16,minor:u16,reader_major:u16,reader_minor:u16,features:Features,
    block_bytes:u32,segment_blocks:u32,volume_blocks:u64,segment_start:u64,segment_count:u64,
    cp0:u64,cp1:u64,cp2:u64,maximum_nodes:u64,maximum_orphans:u64,
    maximum_table_pages:u32,maximum_extents:u32,cleaner_reserve:u64,transaction_bound:u64,
    copy_index:u32,reserved:Zero<4>
}, |v:&Superblock| {
    if (v.major,v.minor,v.reader_major,v.reader_minor)!=(0,2,0,2) { return Err(Error::Unsupported); }
    need(v.block_bytes==4096 && [64,256,512].contains(&v.segment_blocks)
        && (v.cp0,v.cp1,v.cp2)==(2,3,4) && v.copy_index<2
        && v.maximum_nodes>0 && v.maximum_nodes<=6724 && v.maximum_orphans>0
        && v.maximum_orphans<=v.maximum_nodes && (1..=82).contains(&v.maximum_table_pages)
        && (1..=6724).contains(&v.maximum_extents) && v.transaction_bound>0)
});
record!(Checkpoint {
    generation:u64,predecessor:u64,root:NodeId,node_map:BlockRef,segment_table:BlockRef,orphan_table:BlockRef,
    next_nid:u64,allocated_blocks:u64,free_blocks:u64,live_nodes:u64,orphan_nodes:u64,
    transaction_blocks:u64,state:u32,reserved:Zero<4>
}, |v:&Checkpoint| {
    v.root.present()?;v.node_map.present()?;v.segment_table.present()?;v.orphan_table.present()?;
    need(v.generation>0 && v.predecessor<v.generation && v.next_nid>1 && v.state<=1)
});
record!(Descriptor { first:u64,last:u64,page:BlockRef,count:u32,reserved:Zero<4> }, |v:&Descriptor| {
    v.page.present()?; need(v.first<=v.last && v.count>0)
});
record!(MapEntry { id:NodeId,node:BlockRef,kind:u8,reserved:Zero<7> }, |v:&MapEntry| {
    v.id.present()?;v.node.present()?;kind(v.kind)
});
fn kind(v: u8) -> Result<()> {
    if v == 3 {
        Err(Error::Unsupported)
    } else {
        need(v == 1 || v == 2)
    }
}
record!(Inode {
    id:NodeId,kind:u8,encoding:u8,mode:u16,uid:u32,gid:u32,nlink:u32,size:u64,content_generation:u64,
    parent:NodeId,extents:BlockRef,directory:BlockRef,time_valid:u32,reserved0:Zero<4>,
    atime:DiskTime,mtime:DiskTime,ctime:DiskTime,birth:DiskTime,reserved1:Zero<16>
}, |v:&Inode| {
    v.id.present()?;kind(v.kind)?;
    need(v.encoding==0 && v.mode & !0o7777==0 && v.size<=(1u64<<40) && v.content_generation>0 && v.time_valid & !15==0)?;
    if v.kind==2 { v.parent.present()?;v.directory.present()?;need(v.extents.absent())?; }
    else { v.extents.present()?;need(v.parent.absent() && v.directory.absent() && v.nlink<=1)?; }
    for (i,t) in [&v.atime,&v.mtime,&v.ctime,&v.birth].iter().enumerate() {
        if v.time_valid & (1<<i)==0 { need(t.seconds==0 && t.nanos==0)?; }
    }
    Ok(())
});
record!(
    Extent {
        logical: u64,
        physical: u64,
        epoch: u64,
        content_generation: u64,
        blocks: u32,
        flags: u32,
        data_crc: u64
    },
    |v: &Extent| need(
        v.logical < (1u64 << 28)
            && v.physical > 0
            && v.epoch > 0
            && v.content_generation > 0
            && v.blocks == 1
            && v.flags == 0
    )
);
record!(Orphan { id:NodeId,unlink_generation:u64,reason:u16,reserved:Zero<6> }, |v:&Orphan| {
    v.id.present()?;need(v.unlink_generation>0 && v.reason==1)
});
record!(SummaryEntry {
    offset:u32,kind:u16,reserved:Zero<2>,owner:NodeId,logical_key:u64,content_generation:u64,expected_crc:u64
}, |v:&SummaryEntry| {
    need(v.offset<512 && v.kind<=8 && v.content_generation>0)?;
    if v.kind==0 { v.owner.present()?; } else { need(v.expected_crc==0)?; }
    Ok(())
});
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub index: u64,
    pub epoch: u64,
    pub state: u16,
    pub live: u32,
    pub next_hint: u32,
    pub summary: BlockRef,
    pub bitmap: Vec<u8>,
}
impl Field for Segment {
    fn write(&self, e: &mut Encoder) -> Result<()> {
        self.validate()?;
        e.field(&self.index)?;
        e.field(&self.epoch)?;
        e.field(&self.state)?;
        e.field(&Zero::<2>)?;
        e.field(&self.live)?;
        e.field(&self.next_hint)?;
        e.field(&(self.bitmap.len() as u32))?;
        e.field(&self.summary)?;
        e.field(&Zero::<8>)?;
        e.put(&self.bitmap)
    }
    fn read(d: &mut Decoder<'_>) -> Result<Self> {
        let index = d.field()?;
        let epoch = d.field()?;
        let state = d.field()?;
        let _: Zero<2> = d.field()?;
        let live = d.field()?;
        let next_hint = d.field()?;
        let n: u32 = d.field()?;
        if ![8, 32, 64].contains(&n) {
            return Err(Error::Bounds);
        }
        let summary = d.field()?;
        let _: Zero<8> = d.field()?;
        let bitmap = copy(d.take(n as usize)?)?;
        let v = Self {
            index,
            epoch,
            state,
            live,
            next_hint,
            summary,
            bitmap,
        };
        v.validate()?;
        Ok(v)
    }
}
impl Validate for Segment {
    fn validate(&self) -> Result<()> {
        need(
            [8, 32, 64].contains(&self.bitmap.len())
                && self.state <= 3
                && self.next_hint <= self.bitmap.len() as u32 * 8
                && self.bitmap.iter().map(|b| b.count_ones()).sum::<u32>() == self.live,
        )?;
        self.summary.validate()?;
        if self.live == 0 {
            need(self.summary.absent())?;
        } else {
            self.summary.present()?;
            need(self.epoch > 0 && self.state != 0)?;
        }
        if self.state == 0 || self.state == 3 {
            need(self.live == 0)?;
        }
        if self.state != 0 {
            need(self.epoch > 0)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub target: NodeId,
    pub kind: u8,
    pub name: Vec<u8>,
}
impl Validate for DirEntry {
    fn validate(&self) -> Result<()> {
        self.target.present()?;
        kind(self.kind)?;
        need(
            !self.name.is_empty()
                && self.name.len() <= 255
                && self.name != b"."
                && self.name != b".."
                && !self.name.contains(&0)
                && !self.name.contains(&b'/')
                && core::str::from_utf8(&self.name).is_ok(),
        )
    }
}
impl Field for DirEntry {
    fn write(&self, e: &mut Encoder) -> Result<()> {
        self.validate()?;
        let n = self.name.len();
        let total = (32 + n + 7) & !7;
        e.field(&self.target)?;
        e.field(&(n as u16))?;
        e.field(&self.kind)?;
        e.field(&0u8)?;
        e.field(&(total as u16))?;
        e.field(&Zero::<10>)?;
        e.put(&self.name)?;
        e.put(&[0; 7][..total - 32 - n])
    }
    fn read(d: &mut Decoder<'_>) -> Result<Self> {
        let target = d.field()?;
        let n: u16 = d.field()?;
        let kind = d.field()?;
        let flags: u8 = d.field()?;
        let total: u16 = d.field()?;
        let _: Zero<10> = d.field()?;
        need(n > 0 && n <= 255 && flags == 0 && total as usize == ((32 + n as usize + 7) & !7))?;
        let name = copy(d.take(n as usize)?)?;
        if d.take(total as usize - 32 - n as usize)?
            .iter()
            .any(|&v| v != 0)
        {
            return Err(Error::Reserved);
        }
        let v = Self { target, kind, name };
        v.validate()?;
        Ok(v)
    }
}
fn copy(b: &[u8]) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(b.len()).map_err(|_| Error::NoMemory)?;
    v.extend_from_slice(b);
    Ok(v)
}
fn read_many<T: Field>(d: &mut Decoder<'_>, n: usize, min: usize) -> Result<Vec<T>> {
    if n > 512 || n.checked_mul(min).ok_or(Error::Overflow)? > d.remaining() {
        return Err(Error::Bounds);
    }
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| Error::NoMemory)?;
    for _ in 0..n {
        v.push(d.field()?);
    }
    Ok(v)
}
fn write_many<T: Field>(e: &mut Encoder, v: &[T]) -> Result<()> {
    for x in v {
        e.field(x)?;
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Records {
    Map(Vec<MapEntry>),
    Segments(Vec<Segment>),
    Orphans(Vec<Orphan>),
}
impl Records {
    pub fn kind(&self) -> u16 {
        match self {
            Self::Map(_) => 1,
            Self::Segments(_) => 2,
            Self::Orphans(_) => 3,
        }
    }
    pub fn len(&self) -> usize {
        match self {
            Self::Map(v) => v.len(),
            Self::Segments(v) => v.len(),
            Self::Orphans(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn keys(&self) -> Vec<u64> {
        match self {
            Self::Map(v) => v.iter().map(|x| x.id.nid).collect(),
            Self::Segments(v) => v.iter().map(|x| x.index).collect(),
            Self::Orphans(v) => v.iter().map(|x| x.id.nid).collect(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    Superblock(Superblock),
    Checkpoint(Checkpoint),
    Root {
        kind: u16,
        count: u64,
        entry_bytes: u32,
        pages: Vec<Descriptor>,
    },
    Table {
        first: u64,
        last: u64,
        records: Records,
    },
    Inode(Inode),
    Directory {
        owner: NodeId,
        ordinal: u32,
        entries: Vec<DirEntry>,
    },
    Extents {
        owner: NodeId,
        entries: Vec<Extent>,
    },
    Summary {
        segment: u64,
        epoch: u64,
        ordinal: u32,
        next: BlockRef,
        entries: Vec<SummaryEntry>,
    },
}
impl Payload {
    pub fn page_type(&self) -> u16 {
        match self {
            Self::Superblock(_) => 1,
            Self::Checkpoint(_) => 2,
            Self::Root { .. } => 3,
            Self::Table { .. } => 4,
            Self::Inode(_) => 5,
            Self::Directory { .. } => 6,
            Self::Extents { .. } => 7,
            Self::Summary { .. } => 8,
        }
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Superblock(v) => v.validate(),
            Self::Checkpoint(v) => v.validate(),
            Self::Inode(v) => v.validate(),
            Self::Root {
                kind,
                count,
                entry_bytes,
                pages,
            } => {
                need((1..=5).contains(kind) && pages.len() <= 82)?;
                let size = match kind {
                    1 | 5 => 48,
                    2 => *entry_bytes,
                    3 => 32,
                    4 => 0,
                    _ => return Err(Error::Structure),
                };
                need(*entry_bytes == size && (*kind != 2 || [72, 96, 128].contains(entry_bytes)))?;
                let mut sum = 0u64;
                let mut prev = None;
                for (i, p) in pages.iter().enumerate() {
                    p.validate()?;
                    need(prev.is_none_or(|v| p.first > v))?;
                    if *kind == 4 {
                        need(p.first == i as u64 && p.last == i as u64)?;
                    }
                    prev = Some(p.last);
                    sum = sum.checked_add(p.count as u64).ok_or(Error::Overflow)?;
                }
                need(sum == *count)
            }
            Self::Table {
                first,
                last,
                records,
            } => {
                need(!records.is_empty())?;
                let keys = records.keys();
                need(
                    keys.first() == Some(first)
                        && keys.last() == Some(last)
                        && keys.windows(2).all(|w| w[0] < w[1]),
                )?;
                match records {
                    Records::Map(v) => {
                        for x in v {
                            x.validate()?;
                        }
                    }
                    Records::Segments(v) => {
                        for x in v {
                            x.validate()?;
                        }
                    }
                    Records::Orphans(v) => {
                        for x in v {
                            x.validate()?;
                        }
                    }
                }
                Ok(())
            }
            Self::Directory { owner, entries, .. } => {
                owner.present()?;
                need(entries.len() <= 124)?;
                let mut prev: Option<&[u8]> = None;
                for x in entries {
                    x.validate()?;
                    need(prev.is_none_or(|v| v < x.name.as_slice()))?;
                    prev = Some(&x.name);
                }
                Ok(())
            }
            Self::Extents { owner, entries } => {
                owner.present()?;
                need(!entries.is_empty() && entries.len() <= 82)?;
                let mut prev = None;
                for x in entries {
                    x.validate()?;
                    need(prev.is_none_or(|v| v < x.logical))?;
                    prev = Some(x.logical);
                }
                Ok(())
            }
            Self::Summary {
                epoch,
                next,
                entries,
                ..
            } => {
                need(*epoch > 0 && !entries.is_empty() && entries.len() <= 82)?;
                next.validate()?;
                let mut prev = None;
                for x in entries {
                    x.validate()?;
                    need(prev.is_none_or(|v| v < x.offset))?;
                    prev = Some(x.offset);
                }
                Ok(())
            }
        }
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut e = Encoder::new();
        match self {
            Self::Superblock(v) => e.field(v)?,
            Self::Checkpoint(v) => e.field(v)?,
            Self::Inode(v) => e.field(v)?,
            Self::Root {
                kind,
                count,
                entry_bytes,
                pages,
            } => {
                e.field(kind)?;
                e.field(&Zero::<2>)?;
                e.field(&(pages.len() as u32))?;
                e.field(count)?;
                e.field(entry_bytes)?;
                e.field(&Zero::<4>)?;
                write_many(&mut e, pages)?;
            }
            Self::Table {
                first,
                last,
                records,
            } => {
                e.field(&records.kind())?;
                e.field(&Zero::<2>)?;
                e.field(&(records.len() as u32))?;
                e.field(first)?;
                e.field(last)?;
                match records {
                    Records::Map(v) => write_many(&mut e, v)?,
                    Records::Segments(v) => write_many(&mut e, v)?,
                    Records::Orphans(v) => write_many(&mut e, v)?,
                }
            }
            Self::Directory {
                owner,
                ordinal,
                entries,
            } => {
                let mut body = Encoder::new();
                write_many(&mut body, entries)?;
                let body = body.finish();
                e.field(owner)?;
                e.field(ordinal)?;
                e.field(&(entries.len() as u32))?;
                e.field(&(body.len() as u32))?;
                e.field(&Zero::<4>)?;
                e.put(&body)?;
            }
            Self::Extents { owner, entries } => {
                e.field(owner)?;
                e.field(&(entries.len() as u32))?;
                e.field(&Zero::<4>)?;
                write_many(&mut e, entries)?;
            }
            Self::Summary {
                segment,
                epoch,
                ordinal,
                next,
                entries,
            } => {
                e.field(segment)?;
                e.field(epoch)?;
                e.field(ordinal)?;
                e.field(&(entries.len() as u32))?;
                e.field(next)?;
                write_many(&mut e, entries)?;
            }
        }
        Ok(e.finish())
    }
    pub fn decode(page_type: u16, bytes: &[u8]) -> Result<Self> {
        let mut d = Decoder::new(bytes)?;
        let v = match page_type {
            1 => Self::Superblock(d.field()?),
            2 => Self::Checkpoint(d.field()?),
            5 => Self::Inode(d.field()?),
            3 => {
                let kind = d.field()?;
                let _: Zero<2> = d.field()?;
                let n: u32 = d.field()?;
                if n > 82 {
                    return Err(Error::Bounds);
                }
                let count = d.field()?;
                let entry_bytes = d.field()?;
                let _: Zero<4> = d.field()?;
                Self::Root {
                    kind,
                    count,
                    entry_bytes,
                    pages: read_many(&mut d, n as usize, 48)?,
                }
            }
            4 => {
                let kind: u16 = d.field()?;
                let _: Zero<2> = d.field()?;
                let n: u32 = d.field()?;
                let first = d.field()?;
                let last = d.field()?;
                let records = match kind {
                    1 => Records::Map(read_many(&mut d, n as usize, 48)?),
                    2 => Records::Segments(read_many(&mut d, n as usize, 72)?),
                    3 => Records::Orphans(read_many(&mut d, n as usize, 32)?),
                    _ => return Err(Error::Unsupported),
                };
                Self::Table {
                    first,
                    last,
                    records,
                }
            }
            6 => {
                let owner = d.field()?;
                let ordinal = d.field()?;
                let n: u32 = d.field()?;
                let used: u32 = d.field()?;
                let _: Zero<4> = d.field()?;
                need(used as usize == d.remaining())?;
                Self::Directory {
                    owner,
                    ordinal,
                    entries: read_many(&mut d, n as usize, 40)?,
                }
            }
            7 => {
                let owner = d.field()?;
                let n: u32 = d.field()?;
                let _: Zero<4> = d.field()?;
                Self::Extents {
                    owner,
                    entries: read_many(&mut d, n as usize, 48)?,
                }
            }
            8 => {
                let segment = d.field()?;
                let epoch = d.field()?;
                let ordinal = d.field()?;
                let n: u32 = d.field()?;
                let next = d.field()?;
                Self::Summary {
                    segment,
                    epoch,
                    ordinal,
                    next,
                    entries: read_many(&mut d, n as usize, 48)?,
                }
            }
            _ => return Err(Error::Unsupported),
        };
        d.end()?;
        v.validate()?;
        Ok(v)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub uuid: [u8; 16],
    pub block: u64,
    pub epoch: u64,
    pub birth: u64,
    pub payload: Payload,
}
impl Page {
    fn validate(&self) -> Result<()> {
        need(self.uuid != [0; 16])?;
        self.payload.validate()?;
        match &self.payload {
            Payload::Superblock(v) => {
                need(self.block == v.copy_index as u64 && self.epoch == 0 && self.birth == 0)
            }
            Payload::Checkpoint(v) => {
                need((2..=4).contains(&self.block) && self.epoch == 0 && self.birth == v.generation)
            }
            _ => need(self.block >= 64 && self.epoch > 0 && self.birth > 0),
        }
    }
    pub fn encode(&self) -> Result<[u8; PAGE]> {
        self.validate()?;
        let payload = self.payload.encode()?;
        let mut b = [0; PAGE];
        b[..8].copy_from_slice(b"SUNFSM02");
        b[8..10].copy_from_slice(&self.payload.page_type().to_le_bytes());
        b[10..12].copy_from_slice(&2u16.to_le_bytes());
        b[12..14].copy_from_slice(&96u16.to_le_bytes());
        b[16..32].copy_from_slice(&self.uuid);
        b[32..40].copy_from_slice(&self.block.to_le_bytes());
        b[40..48].copy_from_slice(&self.epoch.to_le_bytes());
        b[48..56].copy_from_slice(&self.birth.to_le_bytes());
        b[56..60].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        b[96..96 + payload.len()].copy_from_slice(&payload);
        let crc = crc64(&b);
        b[64..72].copy_from_slice(&crc.to_le_bytes());
        Ok(b)
    }
    pub fn decode(bytes: &[u8], address: u64) -> Result<Self> {
        if bytes.len() != PAGE {
            return Err(Error::Bounds);
        }
        if &bytes[..8] != b"SUNFSM02" {
            return Err(Error::Magic);
        }
        if bytes[10..12] != 2u16.to_le_bytes() {
            return Err(Error::Unsupported);
        }
        if bytes[12..14] != 96u16.to_le_bytes() {
            return Err(Error::Structure);
        }
        if bytes[14..16]
            .iter()
            .chain(&bytes[60..64])
            .chain(&bytes[72..96])
            .any(|&v| v != 0)
        {
            return Err(Error::Reserved);
        }
        let get =
            |a: usize| -> u64 { u64::from_le_bytes(bytes[a..a + 8].try_into().unwrap_or([0; 8])) };
        let payload_len =
            u32::from_le_bytes(bytes[56..60].try_into().map_err(|_| Error::Bounds)?) as usize;
        if payload_len > PAYLOAD {
            return Err(Error::Bounds);
        }
        if bytes[96 + payload_len..].iter().any(|&v| v != 0) {
            return Err(Error::Reserved);
        }
        let mut checked = [0; PAGE];
        checked.copy_from_slice(bytes);
        checked[64..72].fill(0);
        if crc64(&checked) != get(64) {
            return Err(Error::Checksum);
        }
        let block = get(32);
        if block != address {
            return Err(Error::Reference);
        }
        let page_type = u16::from_le_bytes(bytes[8..10].try_into().map_err(|_| Error::Bounds)?);
        let v = Self {
            uuid: bytes[16..32].try_into().map_err(|_| Error::Bounds)?,
            block,
            epoch: get(40),
            birth: get(48),
            payload: Payload::decode(page_type, &bytes[96..96 + payload_len])?,
        };
        v.validate()?;
        Ok(v)
    }
    pub fn reference(&self) -> Result<BlockRef> {
        let bytes = self.encode()?;
        Ok(BlockRef {
            block: self.block,
            epoch: self.epoch,
            crc: u64::from_le_bytes(bytes[64..72].try_into().map_err(|_| Error::Bounds)?),
        })
    }
}
/// Domain-separated raw file-block CRC, per experimental revision 2.
pub fn data_crc(
    uuid: [u8; 16],
    owner: &NodeId,
    logical: u64,
    epoch: u64,
    generation: u64,
    bytes: &[u8; PAGE],
) -> Result<u64> {
    owner.present()?;
    need(uuid != [0; 16] && epoch > 0 && generation > 0)?;
    // Streaming equivalent without a 4152-byte allocation.
    let mut prefix = Encoder::new();
    prefix.field(&uuid)?;
    prefix.field(owner)?;
    prefix.field(&logical)?;
    prefix.field(&epoch)?;
    prefix.field(&generation)?;
    let mut crc = 0u64;
    for &byte in prefix.finish().iter().chain(bytes.iter()) {
        crc ^= (byte as u64) << 56;
        for _ in 0..8 {
            crc = if crc & (1 << 63) != 0 {
                (crc << 1) ^ 0x42f0e1eba9ea3693
            } else {
                crc << 1
            };
        }
    }
    Ok(crc)
}
/// Helper for deterministic fixtures; all production parsing remains bounded.
pub fn empty_root(kind: u16, entry_bytes: u32) -> Payload {
    Payload::Root {
        kind,
        count: 0,
        entry_bytes,
        pages: vec![],
    }
}
