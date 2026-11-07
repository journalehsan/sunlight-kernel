//! USB MSC Bulk-Only Transport, SCSI transparent, LUN 0. No hardware emulation.
//! Transport owns DMA lifetime, completion matching and per-transfer deadlines.
use crate::{BlockDevice, BlockError, BLOCK_SIZE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Descriptor,
    Unsupported,
    SectorSize(u32),
    Timeout,
    Stall,
    Transport,
    Short,
    Status,
    Failed,
    Sense(Sense),
    Offline,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sense {
    pub key: u8,
    pub asc: u8,
    pub ascq: u8,
}
impl Sense {
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        if data.len() < 14
            || !matches!(data[0] & 0x7f, 0x70 | 0x71)
            || data[7] < 6
            || data[7] as usize + 8 > data.len()
        {
            return Err(Error::Status);
        }
        Ok(Self {
            key: data[2] & 15,
            asc: data[12],
            ascq: data[13],
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub address: u8,
    pub max_packet: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interface {
    pub configuration: u8,
    pub number: u8,
    pub input: Endpoint,
    pub output: Endpoint,
}
/// Initial profile: one interface, alternate 0, USB2 high speed, two bulk pipes.
/// Requiring a dedicated device/controller avoids competing ring owners.
pub fn interface(data: &[u8]) -> Result<Interface, Error> {
    if data.len() < 9
        || data[0] != 9
        || data[1] != 2
        || u16::from_le_bytes([data[2], data[3]]) as usize != data.len()
        || data[4] != 1
        || data[5] == 0
        || data[7] & 0x9f != 0x80
    {
        return Err(Error::Descriptor);
    }
    let mut iface = None;
    let mut input = None;
    let mut output = None;
    let mut offset = 9;
    while offset < data.len() {
        if offset + 2 > data.len() {
            return Err(Error::Descriptor);
        }
        let n = data[offset] as usize;
        if n < 2 || offset + n > data.len() {
            return Err(Error::Descriptor);
        }
        let d = &data[offset..offset + n];
        match d[1] {
            4 => {
                if n != 9 || iface.is_some() || d[2] != 0 || d[3] != 0 || d[4] != 2 {
                    return Err(Error::Descriptor);
                }
                if d[5..8] != [8, 6, 0x50] {
                    return Err(Error::Unsupported);
                }
                iface = Some(d[2]);
            }
            5 => {
                if n != 7
                    || iface.is_none()
                    || d[2] & 0x70 != 0
                    || d[2] & 15 == 0
                    || d[3] != 2
                    || u16::from_le_bytes([d[4], d[5]]) != 512
                {
                    return Err(Error::Descriptor);
                }
                let endpoint = Endpoint {
                    address: d[2],
                    max_packet: 512,
                };
                let slot = if d[2] & 0x80 != 0 {
                    &mut input
                } else {
                    &mut output
                };
                if slot.replace(endpoint).is_some() {
                    return Err(Error::Descriptor);
                }
            }
            _ => return Err(Error::Unsupported),
        }
        offset += n;
    }
    Ok(Interface {
        configuration: data[5],
        number: iface.ok_or(Error::Descriptor)?,
        input: input.ok_or(Error::Descriptor)?,
        output: output.ok_or(Error::Descriptor)?,
    })
}

pub trait Transport {
    fn bulk(&mut self, input: bool, data: &mut [u8]) -> Result<usize, Error>;
    fn clear_halt(&mut self, input: bool) -> Result<(), Error>;
    /// Class reset followed by clearing both bulk halts, in that order.
    fn reset(&mut self) -> Result<(), Error>;
}

pub fn cbw(tag: u32, length: usize, input: bool, cdb: &[u8]) -> Result<[u8; 31], Error> {
    if tag == 0 || cdb.is_empty() || cdb.len() > 16 || length > u32::MAX as usize {
        return Err(Error::Status);
    }
    let mut result = [0; 31];
    result[..4].copy_from_slice(&0x43425355u32.to_le_bytes());
    result[4..8].copy_from_slice(&tag.to_le_bytes());
    result[8..12].copy_from_slice(&(length as u32).to_le_bytes());
    result[12] = if input { 0x80 } else { 0 };
    result[14] = cdb.len() as u8; // LUN byte stays 0.
    result[15..15 + cdb.len()].copy_from_slice(cdb);
    Ok(result)
}
pub fn csw(data: &[u8], tag: u32, expected: usize, actual: usize) -> Result<(), Error> {
    if data.len() != 13
        || data[..4] != 0x53425355u32.to_le_bytes()
        || data[4..8] != tag.to_le_bytes()
        || actual > expected
    {
        return Err(Error::Status);
    }
    let residue = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
    if residue > expected {
        return Err(Error::Status);
    }
    match data[12] {
        1 => Err(Error::Failed),
        0 if actual == expected && residue == 0 => Ok(()),
        0 => Err(Error::Short),
        _ => Err(Error::Status),
    }
}

pub struct Bot<T> {
    transport: T,
    next_tag: u32,
    usable: bool,
}
impl<T: Transport> Bot<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            next_tag: 1,
            usable: true,
        }
    }
    pub fn command(&mut self, cdb: &[u8], input: bool, data: &mut [u8]) -> Result<(), Error> {
        if !self.usable {
            return Err(Error::Offline);
        }
        let tag = self.next_tag;
        self.next_tag = self.next_tag.checked_add(1).ok_or(Error::Offline)?;
        let mut wrapper = cbw(tag, data.len(), input, cdb)?;
        let result = self.exchange(&mut wrapper, tag, input, data);
        if matches!(result, Err(e) if e != Error::Failed) {
            // Never replay a possibly executed WRITE. Recover only for a future command.
            if self.transport.reset().is_err() {
                self.usable = false;
            }
        }
        result
    }
    fn exchange(
        &mut self,
        wrapper: &mut [u8; 31],
        tag: u32,
        input: bool,
        data: &mut [u8],
    ) -> Result<(), Error> {
        if self.transport.bulk(false, wrapper)? != 31 {
            return Err(Error::Short);
        }
        let actual = if data.is_empty() {
            0
        } else {
            match self.transport.bulk(input, data) {
                Ok(n) => n,
                Err(Error::Stall) => {
                    self.transport.clear_halt(input)?;
                    0
                }
                Err(e) => return Err(e),
            }
        };
        let mut status = [0u8; 13];
        let n = match self.transport.bulk(true, &mut status) {
            Err(Error::Stall) => {
                self.transport.clear_halt(true)?;
                self.transport.bulk(true, &mut status)?
            }
            result => result?,
        };
        if n > 13 {
            return Err(Error::Status);
        }
        csw(&status[..n], tag, data.len(), actual)
    }
}

pub struct Disk<T> {
    bot: Bot<T>,
    blocks: u64,
    online: bool,
}
impl<T: Transport> Disk<T> {
    pub fn initialize(transport: T) -> Result<Self, Error> {
        let mut disk = Self {
            bot: Bot::new(transport),
            blocks: 0,
            online: true,
        };
        let mut inquiry = [0; 36];
        disk.scsi(&[0x12, 0, 0, 0, 36, 0], true, &mut inquiry)?;
        if inquiry[0] != 0 {
            return Err(Error::Unsupported);
        }
        // Initial unit attention is consumed only before identities/geometry exist.
        let mut ready = false;
        for _ in 0..3 {
            match disk.scsi(&[0; 6], false, &mut []) {
                Ok(()) => {
                    ready = true;
                    break;
                }
                Err(Error::Sense(s)) if s.key == 6 || (s.key == 2 && s.asc == 4) => {}
                Err(e) => return Err(e),
            }
        }
        if !ready {
            return Err(Error::Offline);
        }
        let mut capacity = [0; 8];
        disk.scsi(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], true, &mut capacity)?;
        let sector_size = u32::from_be_bytes(capacity[4..8].try_into().unwrap());
        if sector_size != 512 {
            return Err(Error::SectorSize(sector_size));
        }
        let last = u32::from_be_bytes(capacity[..4].try_into().unwrap());
        if last == u32::MAX {
            return Err(Error::Unsupported);
        }
        disk.blocks = last as u64 + 1;
        Ok(disk)
    }
    fn scsi(&mut self, cdb: &[u8], input: bool, data: &mut [u8]) -> Result<(), Error> {
        match self.bot.command(cdb, input, data) {
            Err(Error::Failed) => {
                let mut bytes = [0; 18];
                self.bot.command(&[3, 0, 0, 0, 18, 0], true, &mut bytes)?;
                Err(Error::Sense(Sense::parse(&bytes)?))
            }
            result => result,
        }
    }
    fn ready(&mut self) -> Result<(), BlockError> {
        if !self.online {
            return Err(BlockError::Io);
        }
        // No cache may sit above removable media without checking generation.
        // Any runtime error permanently retires this incarnation until reboot.
        if self.scsi(&[0; 6], false, &mut []).is_err() {
            self.online = false;
            return Err(BlockError::Io);
        }
        Ok(())
    }
    fn io(&mut self, opcode: u8, lba: u64, bytes: &mut [u8]) -> Result<(), BlockError> {
        if lba >= self.blocks {
            return Err(BlockError::OutOfRange);
        }
        self.ready()?;
        let mut cdb = [0; 10];
        cdb[0] = opcode;
        cdb[2..6].copy_from_slice(&(lba as u32).to_be_bytes());
        cdb[8] = 1;
        if self.scsi(&cdb, opcode == 0x28, bytes).is_err() {
            self.online = false;
            return Err(BlockError::Io);
        }
        Ok(())
    }
}
impl<T: Transport> BlockDevice for Disk<T> {
    fn read_block(&mut self, lba: u64, bytes: &mut [u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        // Failed/short DMA data never becomes caller-visible.
        let mut temporary = [0; BLOCK_SIZE];
        self.io(0x28, lba, &mut temporary)?;
        bytes.copy_from_slice(&temporary);
        Ok(())
    }
    fn write_block(&mut self, lba: u64, bytes: &[u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        self.io(0x2a, lba, &mut bytes.clone())
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.ready()?;
        if self
            .scsi(&[0x35, 0, 0, 0, 0, 0, 0, 0, 0, 0], false, &mut [])
            .is_err()
        {
            self.online = false;
            return Err(BlockError::Io);
        }
        Ok(())
    }
    fn block_count(&self) -> u64 {
        self.blocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, vec, vec::Vec};
    #[derive(Clone)]
    struct Reply {
        opcode: u8,
        data: Vec<u8>,
        status: u8,
        residue: u32,
    }
    struct Fake {
        replies: VecDeque<Reply>,
        current: Option<Reply>,
        tag: u32,
        length: usize,
        input: bool,
        phase: u8,
        resets: usize,
        clears: usize,
        failure: Option<Error>,
        reset_failure: bool,
        short_cbw: bool,
        csw_stall: bool,
    }
    impl Fake {
        fn new(replies: Vec<Reply>) -> Self {
            Self {
                replies: replies.into(),
                current: None,
                tag: 0,
                length: 0,
                input: false,
                phase: 0,
                resets: 0,
                clears: 0,
                failure: None,
                reset_failure: false,
                short_cbw: false,
                csw_stall: false,
            }
        }
    }
    impl Transport for Fake {
        fn bulk(&mut self, input: bool, bytes: &mut [u8]) -> Result<usize, Error> {
            if let Some(e) = self.failure.take() {
                return Err(e);
            }
            match self.phase {
                0 => {
                    assert!(!input);
                    assert_eq!(bytes.len(), 31);
                    assert_eq!(&bytes[..4], b"USBC");
                    assert_eq!(bytes[13], 0);
                    self.tag = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
                    self.length = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
                    self.input = bytes[12] != 0;
                    let reply = self.replies.pop_front().expect("unexpected command");
                    assert_eq!(reply.opcode, bytes[15]);
                    self.current = Some(reply);
                    self.phase = if self.length == 0 { 2 } else { 1 };
                    Ok(if self.short_cbw { 30 } else { 31 })
                }
                1 => {
                    assert_eq!(input, self.input);
                    assert_eq!(bytes.len(), self.length);
                    self.phase = 2;
                    let data = &self.current.as_ref().unwrap().data;
                    if input {
                        bytes[..data.len()].copy_from_slice(data);
                        Ok(data.len())
                    } else {
                        Ok(bytes.len())
                    }
                }
                2 => {
                    assert!(input);
                    assert_eq!(bytes.len(), 13);
                    if self.csw_stall {
                        self.csw_stall = false;
                        return Err(Error::Stall);
                    }
                    let reply = self.current.take().unwrap();
                    bytes[..4].copy_from_slice(b"USBS");
                    bytes[4..8].copy_from_slice(&self.tag.to_le_bytes());
                    bytes[8..12].copy_from_slice(&reply.residue.to_le_bytes());
                    bytes[12] = reply.status;
                    self.phase = 0;
                    Ok(13)
                }
                _ => unreachable!(),
            }
        }
        fn clear_halt(&mut self, _: bool) -> Result<(), Error> {
            self.clears += 1;
            Ok(())
        }
        fn reset(&mut self) -> Result<(), Error> {
            self.resets += 1;
            self.phase = 0;
            if self.reset_failure {
                Err(Error::Timeout)
            } else {
                Ok(())
            }
        }
    }
    fn reply(opcode: u8, data: &[u8], status: u8) -> Reply {
        Reply {
            opcode,
            data: data.into(),
            status,
            residue: 0,
        }
    }
    fn sense(key: u8, asc: u8) -> Vec<u8> {
        let mut bytes = vec![0; 18];
        bytes[0] = 0x70;
        bytes[2] = key;
        bytes[7] = 10;
        bytes[12] = asc;
        bytes
    }
    fn startup() -> Vec<Reply> {
        vec![
            reply(0x12, &[0; 36], 0),
            reply(0, &[], 0),
            reply(0x25, &[0, 0, 0, 31, 0, 0, 2, 0], 0),
        ]
    }
    #[test]
    fn command_wrapper_wire_format_and_bounds() {
        let b = cbw(0x12345678, 512, true, &[0x28; 10]).unwrap();
        assert_eq!(&b[..4], b"USBC");
        assert_eq!(&b[4..8], &[0x78, 0x56, 0x34, 0x12]);
        assert_eq!(&b[8..15], &[0, 2, 0, 0, 0x80, 0, 10]);
        assert_eq!(&b[25..], &[0; 6]);
        assert_eq!(cbw(1, 0, false, &[]), Err(Error::Status));
        assert_eq!(cbw(1, 0, false, &[0; 17]), Err(Error::Status));
        assert_eq!(cbw(0, 0, false, &[0]), Err(Error::Status));
    }
    #[test]
    fn status_signature_tag_length_residue_and_phase_errors() {
        let mut bytes = [0; 13];
        bytes[..4].copy_from_slice(b"USBS");
        bytes[4..8].copy_from_slice(&9u32.to_le_bytes());
        assert_eq!(csw(&bytes, 9, 512, 512), Ok(()));
        assert_eq!(csw(&bytes[..12], 9, 512, 512), Err(Error::Status));
        assert_eq!(csw(&bytes, 8, 512, 512), Err(Error::Status));
        bytes[0] = 0;
        assert_eq!(csw(&bytes, 9, 512, 512), Err(Error::Status));
        bytes[0] = b'U';
        bytes[8..12].copy_from_slice(&513u32.to_le_bytes());
        assert_eq!(csw(&bytes, 9, 512, 512), Err(Error::Status));
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(csw(&bytes, 9, 512, 511), Err(Error::Short));
        bytes[12] = 1;
        assert_eq!(csw(&bytes, 9, 512, 511), Err(Error::Failed));
        for status in [2, 3, 255] {
            bytes[12] = status;
            assert_eq!(csw(&bytes, 9, 512, 511), Err(Error::Status));
        }
        assert_eq!(csw(&bytes, 9, 512, 513), Err(Error::Status));
    }
    #[test]
    fn success_tags_increase_and_failed_status_is_not_transport_reset() {
        let fake = Fake::new(vec![reply(0, &[], 0), reply(0, &[], 1)]);
        let mut bot = Bot::new(fake);
        assert_eq!(bot.command(&[0; 6], false, &mut []), Ok(()));
        assert_eq!(bot.command(&[0; 6], false, &mut []), Err(Error::Failed));
        assert_eq!(bot.transport.tag, 2);
        assert_eq!(bot.transport.resets, 0);
    }
    #[test]
    fn short_data_and_cbw_never_report_success() {
        let mut bot = Bot::new(Fake::new(vec![reply(0x28, &[7; 511], 0)]));
        assert_eq!(
            bot.command(&[0x28; 10], true, &mut [0; 512]),
            Err(Error::Short)
        );
        assert_eq!(bot.transport.resets, 1);
        let mut fake = Fake::new(vec![reply(0, &[], 0)]);
        fake.short_cbw = true;
        let mut bot = Bot::new(fake);
        assert_eq!(bot.command(&[0; 6], false, &mut []), Err(Error::Short));
        assert_eq!(bot.transport.resets, 1);
    }
    #[test]
    fn timeout_bounded_recovery_and_no_replay() {
        let mut fake = Fake::new(vec![reply(0x2a, &[], 0)]);
        fake.failure = Some(Error::Timeout);
        fake.reset_failure = true;
        let mut bot = Bot::new(fake);
        assert_eq!(
            bot.command(&[0x2a; 10], false, &mut [9; 512]),
            Err(Error::Timeout)
        );
        assert_eq!(
            bot.command(&[0x2a; 10], false, &mut [9; 512]),
            Err(Error::Offline)
        );
        assert_eq!(bot.transport.resets, 1);
        assert_eq!(bot.transport.replies.len(), 1);
    }
    #[test]
    fn csw_stall_cleared_and_retried_once() {
        let mut fake = Fake::new(vec![reply(0, &[], 0)]);
        fake.csw_stall = true;
        let mut bot = Bot::new(fake);
        assert_eq!(bot.command(&[0; 6], false, &mut []), Ok(()));
        assert_eq!(bot.transport.clears, 1);
        assert_eq!(bot.transport.resets, 0);
    }
    #[test]
    fn tag_exhaustion_does_not_wrap() {
        let mut bot = Bot::new(Fake::new(vec![]));
        bot.next_tag = u32::MAX;
        assert_eq!(bot.command(&[0], false, &mut []), Err(Error::Offline));
    }
    #[test]
    fn scsi_initialization_read_write_sync_and_bounds() {
        let mut replies = startup();
        replies.extend([
            reply(0, &[], 0),
            reply(0x28, &[0x5a; 512], 0),
            reply(0, &[], 0),
            reply(0x2a, &[], 0),
            reply(0, &[], 0),
            reply(0x35, &[], 0),
        ]);
        let mut disk = Disk::initialize(Fake::new(replies)).unwrap();
        assert_eq!(disk.block_count(), 32);
        let mut bytes = [0; 512];
        disk.read_block(31, &mut bytes).unwrap();
        assert_eq!(bytes, [0x5a; 512]);
        disk.write_block(31, &bytes).unwrap();
        disk.flush().unwrap();
        assert_eq!(disk.read_block(32, &mut bytes), Err(BlockError::OutOfRange));
        assert!(disk.bot.transport.replies.is_empty());
    }
    #[test]
    fn reject_4kn_and_read_capacity_16_sentinel() {
        for peripheral in [5, 0x20, 0x60, 0x80] {
            let mut replies = startup();
            replies[0].data[0] = peripheral;
            assert!(matches!(
                Disk::initialize(Fake::new(replies)),
                Err(Error::Unsupported)
            ));
        }
        for bytes in [[0, 0, 0, 31, 0, 0, 16, 0], [255, 255, 255, 255, 0, 0, 2, 0]] {
            let mut replies = startup();
            replies[2] = reply(0x25, &bytes, 0);
            assert!(matches!(
                Disk::initialize(Fake::new(replies)),
                Err(Error::Unsupported | Error::SectorSize(4096))
            ));
        }
    }
    #[test]
    fn initial_unit_attention_is_bounded_and_consumed_before_registration() {
        let mut replies = startup();
        replies.splice(
            1..2,
            [
                reply(0, &[], 1),
                reply(3, &sense(6, 0x29), 0),
                reply(0, &[], 0),
            ],
        );
        assert!(Disk::initialize(Fake::new(replies)).is_ok());
    }
    #[test]
    fn runtime_not_ready_and_unit_attention_retire_media_and_preserve_read_buffer() {
        for (key, asc) in [(2, 0x3a), (6, 0x28)] {
            let mut replies = startup();
            replies.extend([reply(0, &[], 1), reply(3, &sense(key, asc), 0)]);
            let mut disk = Disk::initialize(Fake::new(replies)).unwrap();
            let mut bytes = [0xee; 512];
            assert_eq!(disk.read_block(0, &mut bytes), Err(BlockError::Io));
            assert_eq!(bytes, [0xee; 512]);
            assert_eq!(disk.write_block(0, &bytes), Err(BlockError::Io));
            assert_eq!(disk.flush(), Err(BlockError::Io));
            assert!(disk.bot.transport.replies.is_empty());
        }
    }
    #[test]
    fn short_scsi_read_and_sense_failure_do_not_publish_stale_bytes() {
        let mut replies = startup();
        replies.extend([reply(0, &[], 0), reply(0x28, &[4; 511], 0)]);
        let mut disk = Disk::initialize(Fake::new(replies)).unwrap();
        let mut bytes = [0xee; 512];
        assert_eq!(disk.read_block(0, &mut bytes), Err(BlockError::Io));
        assert_eq!(bytes, [0xee; 512]);
        assert_eq!(Sense::parse(&[]), Err(Error::Status));
        assert_eq!(Sense::parse(&[0; 18]), Err(Error::Status));
        assert_eq!(
            Sense::parse(&sense(2, 0x3a)),
            Ok(Sense {
                key: 2,
                asc: 0x3a,
                ascq: 0
            })
        );
    }
    fn descriptors() -> Vec<u8> {
        vec![
            9, 2, 32, 0, 1, 1, 0, 0x80, 50, 9, 4, 0, 0, 2, 8, 6, 0x50, 0, 7, 5, 0x81, 2, 0, 2, 0,
            7, 5, 2, 2, 0, 2, 0,
        ]
    }
    #[test]
    fn interface_bulk_endpoint_validation() {
        let valid = descriptors();
        let i = interface(&valid).unwrap();
        assert_eq!(i.input.address, 0x81);
        assert_eq!(i.output.address, 2);
        for (offset, value) in [
            (0, 8),
            (2, 33),
            (4, 2),
            (5, 0),
            (7, 0),
            (7, 0x81),
            (11, 1),
            (9, 0),
            (12, 1),
            (13, 3),
            (14, 3),
            (15, 5),
            (16, 0x62),
            (18, 6),
            (20, 0x80),
            (20, 0x91),
            (21, 3),
            (22, 1),
            (23, 4),
            (27, 0x82),
            (25, 0),
        ] {
            let mut bad = valid.clone();
            bad[offset] = value;
            assert!(interface(&bad).is_err(), "offset={offset} value={value}");
        }
        for length in 0..valid.len() {
            assert!(interface(&valid[..length]).is_err());
        }
    }
}
