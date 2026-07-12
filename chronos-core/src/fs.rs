//! Guest-facing DOS storage.  This module deliberately stores only DOS paths
//! and bytes; host paths, descriptors, and capabilities stay in the native
//! Chronos adapter.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::{String, ToString},
    vec,
    vec::Vec,
};

pub const MAX_DOS_PATH: usize = 240;
pub const MAX_DOS_COMPONENT: usize = 63;
pub const MAX_OPEN_HANDLES: usize = 64;
pub const MAX_SEARCH_RESULTS: usize = 512;

pub const ATTR_READ_ONLY: u8 = 0x01;
pub const ATTR_HIDDEN: u8 = 0x02;
pub const ATTR_SYSTEM: u8 = 0x04;
pub const ATTR_DIRECTORY: u8 = 0x10;
pub const ATTR_ARCHIVE: u8 = 0x20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum DosError {
    FileNotFound = 2,
    PathNotFound = 3,
    TooManyOpenFiles = 4,
    AccessDenied = 5,
    InvalidHandle = 6,
    InsufficientMemory = 8,
    InvalidDrive = 15,
    NoMoreFiles = 18,
}

impl DosError {
    pub const fn code(self) -> u16 {
        self as u16
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DosDrive {
    C,
    D,
    T,
}

impl DosDrive {
    pub const fn number(self) -> u8 {
        match self {
            Self::C => 2,
            Self::D => 3,
            Self::T => 19,
        }
    }

    pub const fn letter(self) -> u8 {
        match self {
            Self::C => b'C',
            Self::D => b'D',
            Self::T => b'T',
        }
    }

    pub fn from_number(number: u8) -> Option<Self> {
        match number {
            2 => Some(Self::C),
            3 => Some(Self::D),
            19 => Some(Self::T),
            _ => None,
        }
    }

    fn from_ascii(byte: u8) -> Option<Self> {
        match byte.to_ascii_uppercase() {
            b'C' => Some(Self::C),
            b'D' => Some(Self::D),
            b'T' => Some(Self::T),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveAccess {
    ReadOnly,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

impl OpenMode {
    pub const fn can_read(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite)
    }

    pub const fn can_write(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }

    pub fn from_dos(value: u8) -> Result<Self, DosError> {
        match value & 0x03 {
            0 => Ok(Self::ReadOnly),
            1 => Ok(Self::WriteOnly),
            2 => Ok(Self::ReadWrite),
            _ => Err(DosError::AccessDenied),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DosPath {
    pub drive: DosDrive,
    /// Uppercase, `/`-separated path relative to a mounted root.  Empty is
    /// the DOS drive root and is never allowed to escape above that root.
    pub relative: String,
}

impl DosPath {
    pub fn parse(
        input: &[u8],
        current_drive: DosDrive,
        current_directories: &BTreeMap<DosDrive, String>,
    ) -> Result<Self, DosError> {
        if input.is_empty() || input.len() > MAX_DOS_PATH || input.contains(&0) {
            return Err(DosError::PathNotFound);
        }

        let mut index = 0usize;
        let mut drive = current_drive;
        let absolute = if input.len() >= 2 && input[1] == b':' {
            drive = DosDrive::from_ascii(input[0]).ok_or(DosError::InvalidDrive)?;
            index = 2;
            input.get(index).is_some_and(|byte| is_separator(*byte))
        } else if input[0] == b':' {
            return Err(DosError::InvalidDrive);
        } else {
            is_separator(input[0])
        };

        let mut components = if absolute {
            Vec::new()
        } else {
            current_directories
                .get(&drive)
                .map(|path| split_components(path.as_bytes()))
                .unwrap_or_default()
        };

        while index < input.len() && is_separator(input[index]) {
            index += 1;
        }
        let tail = &input[index..];
        for raw in tail.split(|byte| is_separator(*byte)) {
            if raw.is_empty() || raw == b"." {
                continue;
            }
            if raw == b".." {
                if components.pop().is_none() {
                    return Err(DosError::PathNotFound);
                }
                continue;
            }
            if raw.len() > MAX_DOS_COMPONENT
                || raw.iter().any(|byte| !is_valid_component_byte(*byte))
                || raw.contains(&b':')
            {
                return Err(DosError::PathNotFound);
            }
            let mut component = String::with_capacity(raw.len());
            for byte in raw {
                component.push(byte.to_ascii_uppercase() as char);
            }
            if is_reserved_device(component.as_bytes()) {
                return Err(DosError::AccessDenied);
            }
            components.push(component);
        }

        let relative = join_components(&components);
        if relative.len() > MAX_DOS_PATH {
            return Err(DosError::PathNotFound);
        }
        Ok(Self { drive, relative })
    }

    pub fn display(&self) -> String {
        let mut output = String::new();
        output.push(self.drive.letter() as char);
        output.push(':');
        output.push('\\');
        for (index, component) in self
            .relative
            .split('/')
            .filter(|part| !part.is_empty())
            .enumerate()
        {
            if index != 0 {
                output.push('\\');
            }
            output.push_str(component);
        }
        output
    }

    pub fn parent(&self) -> String {
        match self.relative.rsplit_once('/') {
            Some((parent, _)) => parent.to_string(),
            None => String::new(),
        }
    }

    pub fn filename(&self) -> &str {
        self.relative.rsplit('/').next().unwrap_or("")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DosEntry {
    pub data: Vec<u8>,
    pub attributes: u8,
    pub is_directory: bool,
}

impl DosEntry {
    pub fn file(data: Vec<u8>, attributes: u8) -> Self {
        Self {
            data,
            attributes: attributes | ATTR_ARCHIVE,
            is_directory: false,
        }
    }

    pub fn directory() -> Self {
        Self {
            data: Vec::new(),
            attributes: ATTR_DIRECTORY,
            is_directory: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MountedDrive {
    pub access: DriveAccess,
    pub current_directory: String,
    /// Read-only Program + Dependencies content.  `C:` uses this map; direct
    /// launches may use it with an empty persistent overlay.
    base: BTreeMap<String, DosEntry>,
    overlay: BTreeMap<String, DosEntry>,
    tombstones: BTreeSet<String>,
}

impl MountedDrive {
    fn new(access: DriveAccess) -> Self {
        Self {
            access,
            current_directory: String::new(),
            base: BTreeMap::new(),
            overlay: BTreeMap::new(),
            tombstones: BTreeSet::new(),
        }
    }

    fn visible_entry(&self, path: &str) -> Option<&DosEntry> {
        if self.is_tombstoned(path) {
            return None;
        }
        self.overlay.get(path).or_else(|| self.base.get(path))
    }

    fn is_tombstoned(&self, path: &str) -> bool {
        self.tombstones
            .iter()
            .any(|removed| removed == path || path.starts_with(&(removed.clone() + "/")))
    }

    fn ensure_writable(&self) -> Result<(), DosError> {
        if self.access == DriveAccess::ReadWrite {
            Ok(())
        } else {
            Err(DosError::AccessDenied)
        }
    }

    fn parent_is_directory(&self, path: &DosPath) -> Result<(), DosError> {
        let parent = path.parent();
        if parent.is_empty() {
            return Ok(());
        }
        match self.visible_entry(&parent) {
            Some(entry) if entry.is_directory => Ok(()),
            Some(_) => Err(DosError::PathNotFound),
            None => Err(DosError::PathNotFound),
        }
    }

    fn copy_up(&mut self, path: &str) -> Result<(), DosError> {
        if self.overlay.contains_key(path) {
            return Ok(());
        }
        let Some(entry) = self.base.get(path) else {
            return Err(DosError::FileNotFound);
        };
        self.overlay.insert(path.to_string(), entry.clone());
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct DriveTable {
    pub current_drive: DosDrive,
    drives: BTreeMap<DosDrive, MountedDrive>,
}

impl Default for DriveTable {
    fn default() -> Self {
        Self::new()
    }
}

impl DriveTable {
    pub fn new() -> Self {
        let mut drives = BTreeMap::new();
        drives.insert(DosDrive::C, MountedDrive::new(DriveAccess::ReadWrite));
        drives.insert(DosDrive::D, MountedDrive::new(DriveAccess::ReadWrite));
        drives.insert(DosDrive::T, MountedDrive::new(DriveAccess::ReadWrite));
        Self {
            current_drive: DosDrive::C,
            drives,
        }
    }

    pub fn set_access(&mut self, drive: DosDrive, access: DriveAccess) {
        if let Some(mounted) = self.drives.get_mut(&drive) {
            mounted.access = access;
        }
    }

    pub fn mounted_count(&self) -> u8 {
        self.drives.len().min(u8::MAX as usize) as u8
    }

    pub fn select(&mut self, drive: DosDrive) -> Result<(), DosError> {
        if self.drives.contains_key(&drive) {
            self.current_drive = drive;
            Ok(())
        } else {
            Err(DosError::InvalidDrive)
        }
    }

    pub fn current_directory(&self, drive: DosDrive) -> Result<&str, DosError> {
        self.drives
            .get(&drive)
            .map(|mounted| mounted.current_directory.as_str())
            .ok_or(DosError::InvalidDrive)
    }

    pub fn change_directory(&mut self, path: &DosPath) -> Result<(), DosError> {
        let mounted = self
            .drives
            .get_mut(&path.drive)
            .ok_or(DosError::InvalidDrive)?;
        let entry = mounted
            .visible_entry(&path.relative)
            .ok_or(DosError::PathNotFound)?;
        if !entry.is_directory {
            return Err(DosError::PathNotFound);
        }
        mounted.current_directory = path.relative.clone();
        Ok(())
    }

    pub fn parse_path(&self, input: &[u8]) -> Result<DosPath, DosError> {
        let mut directories = BTreeMap::new();
        for (&drive, mounted) in &self.drives {
            directories.insert(drive, mounted.current_directory.clone());
        }
        DosPath::parse(input, self.current_drive, &directories)
    }

    pub fn add_base_file(
        &mut self,
        drive: DosDrive,
        path: &str,
        data: Vec<u8>,
    ) -> Result<(), DosError> {
        self.add_base_entry(drive, path, DosEntry::file(data, 0))
    }

    pub fn add_base_directory(&mut self, drive: DosDrive, path: &str) -> Result<(), DosError> {
        self.add_base_entry(drive, path, DosEntry::directory())
    }

    pub fn add_base_entry(
        &mut self,
        drive: DosDrive,
        path: &str,
        entry: DosEntry,
    ) -> Result<(), DosError> {
        let path = self.parse_external_path(drive, path)?;
        let mounted = self.drives.get_mut(&drive).ok_or(DosError::InvalidDrive)?;
        if mounted.base.contains_key(&path) {
            return Err(DosError::AccessDenied);
        }
        if !path.is_empty() {
            let parent = parent_of(&path);
            if !parent.is_empty()
                && !mounted
                    .base
                    .get(&parent)
                    .is_some_and(|parent| parent.is_directory)
            {
                return Err(DosError::PathNotFound);
            }
        }
        mounted.base.insert(path, entry);
        Ok(())
    }

    pub fn import_overlay_entry(
        &mut self,
        drive: DosDrive,
        path: &str,
        entry: DosEntry,
    ) -> Result<(), DosError> {
        let path = self.parse_external_path(drive, path)?;
        let mounted = self.drives.get_mut(&drive).ok_or(DosError::InvalidDrive)?;
        mounted.overlay.insert(path, entry);
        Ok(())
    }

    pub fn overlay_entries(&self, drive: DosDrive) -> Result<Vec<(String, DosEntry)>, DosError> {
        let mounted = self.drives.get(&drive).ok_or(DosError::InvalidDrive)?;
        Ok(mounted
            .overlay
            .iter()
            .map(|(path, entry)| (path.clone(), entry.clone()))
            .collect())
    }

    pub fn tombstones(&self, drive: DosDrive) -> Result<Vec<String>, DosError> {
        Ok(self
            .drives
            .get(&drive)
            .ok_or(DosError::InvalidDrive)?
            .tombstones
            .iter()
            .cloned()
            .collect())
    }

    pub fn read_file(&self, path: &DosPath) -> Result<&[u8], DosError> {
        let mounted = self.drives.get(&path.drive).ok_or(DosError::InvalidDrive)?;
        let entry = mounted
            .visible_entry(&path.relative)
            .ok_or(DosError::FileNotFound)?;
        if entry.is_directory {
            return Err(DosError::AccessDenied);
        }
        Ok(&entry.data)
    }

    pub fn create_file(&mut self, path: &DosPath, attributes: u8) -> Result<(), DosError> {
        let mounted = self
            .drives
            .get_mut(&path.drive)
            .ok_or(DosError::InvalidDrive)?;
        mounted.ensure_writable()?;
        mounted.parent_is_directory(path)?;
        if path.relative.is_empty() {
            return Err(DosError::AccessDenied);
        }
        mounted.tombstones.remove(&path.relative);
        mounted.overlay.insert(
            path.relative.clone(),
            DosEntry::file(Vec::new(), attributes),
        );
        Ok(())
    }

    pub fn open_for_write(&mut self, path: &DosPath) -> Result<(), DosError> {
        let mounted = self
            .drives
            .get_mut(&path.drive)
            .ok_or(DosError::InvalidDrive)?;
        mounted.ensure_writable()?;
        let entry = mounted
            .visible_entry(&path.relative)
            .ok_or(DosError::FileNotFound)?;
        if entry.is_directory || entry.attributes & ATTR_READ_ONLY != 0 {
            return Err(DosError::AccessDenied);
        }
        mounted.copy_up(&path.relative)?;
        Ok(())
    }

    pub fn write_file(
        &mut self,
        path: &DosPath,
        position: usize,
        data: &[u8],
        truncate_at_position: bool,
    ) -> Result<usize, DosError> {
        self.open_for_write(path)?;
        let mounted = self
            .drives
            .get_mut(&path.drive)
            .ok_or(DosError::InvalidDrive)?;
        let entry = mounted
            .overlay
            .get_mut(&path.relative)
            .ok_or(DosError::FileNotFound)?;
        if truncate_at_position {
            entry.data.truncate(position);
        }
        if position > entry.data.len() {
            entry.data.resize(position, 0);
        }
        let end = position
            .checked_add(data.len())
            .ok_or(DosError::InsufficientMemory)?;
        if end > entry.data.len() {
            entry.data.resize(end, 0);
        }
        entry.data[position..end].copy_from_slice(data);
        entry.attributes |= ATTR_ARCHIVE;
        Ok(data.len())
    }

    pub fn file_len(&self, path: &DosPath) -> Result<usize, DosError> {
        Ok(self.read_file(path)?.len())
    }

    pub fn delete_file(&mut self, path: &DosPath) -> Result<(), DosError> {
        let mounted = self
            .drives
            .get_mut(&path.drive)
            .ok_or(DosError::InvalidDrive)?;
        mounted.ensure_writable()?;
        let entry = mounted
            .visible_entry(&path.relative)
            .ok_or(DosError::FileNotFound)?;
        if entry.is_directory {
            return Err(DosError::AccessDenied);
        }
        mounted.overlay.remove(&path.relative);
        if mounted.base.contains_key(&path.relative) {
            mounted.tombstones.insert(path.relative.clone());
        }
        Ok(())
    }

    pub fn mkdir(&mut self, path: &DosPath) -> Result<(), DosError> {
        let mounted = self
            .drives
            .get_mut(&path.drive)
            .ok_or(DosError::InvalidDrive)?;
        mounted.ensure_writable()?;
        mounted.parent_is_directory(path)?;
        if path.relative.is_empty() || mounted.visible_entry(&path.relative).is_some() {
            return Err(DosError::AccessDenied);
        }
        mounted.tombstones.remove(&path.relative);
        mounted
            .overlay
            .insert(path.relative.clone(), DosEntry::directory());
        Ok(())
    }

    pub fn rmdir(&mut self, path: &DosPath) -> Result<(), DosError> {
        if path.relative.is_empty() {
            return Err(DosError::AccessDenied);
        }
        let mounted = self
            .drives
            .get_mut(&path.drive)
            .ok_or(DosError::InvalidDrive)?;
        mounted.ensure_writable()?;
        let entry = mounted
            .visible_entry(&path.relative)
            .ok_or(DosError::PathNotFound)?;
        if !entry.is_directory {
            return Err(DosError::PathNotFound);
        }
        let prefix = path.relative.clone() + "/";
        if mounted
            .overlay
            .keys()
            .chain(mounted.base.keys())
            .any(|name| name.starts_with(&prefix) && !mounted.is_tombstoned(name))
        {
            return Err(DosError::AccessDenied);
        }
        mounted.overlay.remove(&path.relative);
        if mounted.base.contains_key(&path.relative) {
            mounted.tombstones.insert(path.relative.clone());
        }
        Ok(())
    }

    pub fn rename(&mut self, old: &DosPath, new: &DosPath) -> Result<(), DosError> {
        if old.drive != new.drive || old.relative.is_empty() || new.relative.is_empty() {
            return Err(DosError::AccessDenied);
        }
        let mounted = self
            .drives
            .get_mut(&old.drive)
            .ok_or(DosError::InvalidDrive)?;
        mounted.ensure_writable()?;
        mounted.parent_is_directory(new)?;
        if mounted.visible_entry(&new.relative).is_some() {
            return Err(DosError::AccessDenied);
        }
        let entry = mounted
            .visible_entry(&old.relative)
            .ok_or(DosError::FileNotFound)?;
        if entry.is_directory {
            return Err(DosError::AccessDenied);
        }
        let copy = entry.clone();
        mounted.overlay.insert(new.relative.clone(), copy);
        mounted.overlay.remove(&old.relative);
        if mounted.base.contains_key(&old.relative) {
            mounted.tombstones.insert(old.relative.clone());
        }
        Ok(())
    }

    pub fn get_attributes(&self, path: &DosPath) -> Result<u8, DosError> {
        self.drives
            .get(&path.drive)
            .ok_or(DosError::InvalidDrive)?
            .visible_entry(&path.relative)
            .map(|entry| entry.attributes)
            .ok_or(DosError::FileNotFound)
    }

    pub fn set_attributes(&mut self, path: &DosPath, attributes: u8) -> Result<(), DosError> {
        self.open_for_write(path)?;
        let mounted = self
            .drives
            .get_mut(&path.drive)
            .ok_or(DosError::InvalidDrive)?;
        let entry = mounted
            .overlay
            .get_mut(&path.relative)
            .ok_or(DosError::FileNotFound)?;
        let directory = entry.attributes & ATTR_DIRECTORY;
        entry.attributes = directory | (attributes & !ATTR_DIRECTORY);
        Ok(())
    }

    pub fn list(
        &self,
        directory: &DosPath,
        pattern: &[u8],
        attribute_mask: u16,
    ) -> Result<Vec<DirectoryEntry>, DosError> {
        let mounted = self
            .drives
            .get(&directory.drive)
            .ok_or(DosError::InvalidDrive)?;
        if !directory.relative.is_empty()
            && !mounted
                .visible_entry(&directory.relative)
                .is_some_and(|entry| entry.is_directory)
        {
            return Err(DosError::PathNotFound);
        }
        let prefix = if directory.relative.is_empty() {
            String::new()
        } else {
            directory.relative.clone() + "/"
        };
        let mut results = BTreeMap::<String, DirectoryEntry>::new();
        for (path, entry) in mounted.base.iter().chain(mounted.overlay.iter()) {
            if mounted.is_tombstoned(path) || !path.starts_with(&prefix) {
                continue;
            }
            let suffix = &path[prefix.len()..];
            if suffix.is_empty() || suffix.contains('/') {
                continue;
            }
            if !attributes_visible(entry.attributes, attribute_mask)
                || !wildcard_matches(pattern, suffix.as_bytes())
            {
                continue;
            }
            results.insert(
                suffix.to_string(),
                DirectoryEntry {
                    name: suffix.to_string(),
                    attributes: entry.attributes,
                    size: entry.data.len().min(u32::MAX as usize) as u32,
                },
            );
        }
        let output: Vec<_> = results.into_values().take(MAX_SEARCH_RESULTS).collect();
        Ok(output)
    }

    fn parse_external_path(&self, drive: DosDrive, path: &str) -> Result<String, DosError> {
        let mut current = BTreeMap::new();
        current.insert(drive, String::new());
        let mut encoded = Vec::with_capacity(path.len() + 3);
        encoded.push(drive.letter());
        encoded.push(b':');
        encoded.push(b'\\');
        encoded.extend_from_slice(path.as_bytes());
        Ok(DosPath::parse(&encoded, drive, &current)?.relative)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub name: String,
    pub attributes: u8,
    pub size: u32,
}

#[derive(Clone, Debug)]
pub struct DosHandle {
    pub path: DosPath,
    pub position: usize,
    pub mode: OpenMode,
}

#[derive(Clone, Debug)]
pub struct DosHandleTable {
    entries: Vec<Option<DosHandle>>,
}

impl Default for DosHandleTable {
    fn default() -> Self {
        Self::new()
    }
}

impl DosHandleTable {
    pub fn new() -> Self {
        Self {
            entries: (0..MAX_OPEN_HANDLES).map(|_| None).collect(),
        }
    }

    pub fn open(&mut self, path: DosPath, mode: OpenMode) -> Result<u16, DosError> {
        for index in 5..self.entries.len() {
            if self.entries[index].is_none() {
                self.entries[index] = Some(DosHandle {
                    path,
                    position: 0,
                    mode,
                });
                return Ok(index as u16);
            }
        }
        Err(DosError::TooManyOpenFiles)
    }

    pub fn get(&self, handle: u16) -> Result<&DosHandle, DosError> {
        self.entries
            .get(handle as usize)
            .and_then(Option::as_ref)
            .ok_or(DosError::InvalidHandle)
    }

    pub fn get_mut(&mut self, handle: u16) -> Result<&mut DosHandle, DosError> {
        self.entries
            .get_mut(handle as usize)
            .and_then(Option::as_mut)
            .ok_or(DosError::InvalidHandle)
    }

    pub fn close(&mut self, handle: u16) -> Result<(), DosError> {
        let slot = self
            .entries
            .get_mut(handle as usize)
            .ok_or(DosError::InvalidHandle)?;
        if slot.take().is_some() {
            Ok(())
        } else {
            Err(DosError::InvalidHandle)
        }
    }
}

pub fn wildcard_matches(pattern: &[u8], name: &[u8]) -> bool {
    let pattern = normalize_wildcard(pattern);
    let name = name
        .iter()
        .map(|byte| byte.to_ascii_uppercase())
        .collect::<Vec<_>>();
    wildcard_match(&pattern, &name)
}

fn wildcard_match(pattern: &[u8], name: &[u8]) -> bool {
    let (mut p, mut n, mut star, mut retry) = (0usize, 0usize, None, 0usize);
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            retry = n;
        } else if let Some(position) = star {
            p = position + 1;
            retry += 1;
            n = retry;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

fn normalize_wildcard(pattern: &[u8]) -> Vec<u8> {
    if pattern == b"*.*" {
        return vec![b'*'];
    }
    pattern
        .iter()
        .map(|byte| byte.to_ascii_uppercase())
        .collect()
}

fn attributes_visible(attributes: u8, mask: u16) -> bool {
    let requested = mask as u8;
    if attributes & ATTR_DIRECTORY != 0 && requested & ATTR_DIRECTORY == 0 {
        return false;
    }
    if attributes & ATTR_HIDDEN != 0 && requested & ATTR_HIDDEN == 0 {
        return false;
    }
    if attributes & ATTR_SYSTEM != 0 && requested & ATTR_SYSTEM == 0 {
        return false;
    }
    true
}

fn split_components(path: &[u8]) -> Vec<String> {
    path.split(|byte| *byte == b'/')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut output = String::with_capacity(part.len());
            for byte in part {
                output.push(*byte as char);
            }
            output
        })
        .collect()
}

fn join_components(components: &[String]) -> String {
    let mut output = String::new();
    for (index, component) in components.iter().enumerate() {
        if index != 0 {
            output.push('/');
        }
        output.push_str(component);
    }
    output
}

fn parent_of(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

fn is_separator(byte: u8) -> bool {
    byte == b'\\' || byte == b'/'
}

fn is_valid_component_byte(byte: u8) -> bool {
    byte.is_ascii_graphic()
        && !matches!(
            byte,
            b'<' | b'>' | b'"' | b'|' | b':' | b'+' | b'=' | b';' | b','
        )
}

pub fn is_reserved_device(name: &[u8]) -> bool {
    let base = name.split(|byte| *byte == b'.').next().unwrap_or(name);
    matches!(base, b"CON" | b"NUL" | b"PRN" | b"AUX")
        || (base.len() == 4
            && matches!(&base[..3], b"COM" | b"LPT")
            && matches!(base[3], b'1'..=b'9'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(table: &DriveTable, value: &[u8]) -> DosPath {
        table.parse_path(value).unwrap()
    }

    #[test]
    fn path_preserves_drive_relative_and_absolute_meaning() {
        let mut table = DriveTable::new();
        table.add_base_directory(DosDrive::C, "SUB").unwrap();
        table.change_directory(&path(&table, b"C:\\SUB")).unwrap();

        assert_eq!(path(&table, b"C:FILE.TXT").relative, "SUB/FILE.TXT");
        assert_eq!(path(&table, b"C:\\FILE.TXT").relative, "FILE.TXT");
        assert_eq!(path(&table, b"\\FILE.TXT").relative, "FILE.TXT");
    }

    #[test]
    fn rejects_root_escape_and_reserved_devices() {
        let table = DriveTable::new();
        assert_eq!(
            table.parse_path(b"C:\\..\\SECRET"),
            Err(DosError::PathNotFound)
        );
        assert_eq!(table.parse_path(b"C:\\CON"), Err(DosError::AccessDenied));
        assert!(is_reserved_device(b"COM1.TXT"));
    }

    #[test]
    fn overlay_copy_up_tombstones_and_rename_preserve_base() {
        let mut table = DriveTable::new();
        table
            .add_base_file(DosDrive::C, "README.TXT", b"base".to_vec())
            .unwrap();
        let readme = path(&table, b"C:\\README.TXT");
        table.write_file(&readme, 0, b"overlay", false).unwrap();
        assert_eq!(table.read_file(&readme).unwrap(), b"overlay");

        let renamed = path(&table, b"C:\\RENAMED.TXT");
        table.rename(&readme, &renamed).unwrap();
        assert_eq!(table.read_file(&renamed).unwrap(), b"overlay");
        assert_eq!(table.read_file(&readme), Err(DosError::FileNotFound));
        assert_eq!(
            table
                .overlay_entries(DosDrive::C)
                .unwrap()
                .into_iter()
                .find(|(name, _)| name == "README.TXT"),
            None
        );
    }

    #[test]
    fn aliases_are_case_insensitive_and_wildcards_are_deterministic() {
        let mut table = DriveTable::new();
        table
            .add_base_file(DosDrive::D, "CHRONOS.TXT", Vec::new())
            .unwrap();
        table
            .add_base_file(DosDrive::D, "NOTES.DOC", Vec::new())
            .unwrap();
        let entries = table.list(&path(&table, b"D:\\"), b"*.txt", 0).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "CHRONOS.TXT");
        assert!(wildcard_matches(b"*.*", b"CHRONOS"));
    }
}
