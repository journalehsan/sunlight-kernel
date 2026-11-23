use super::{BlockDevice, FsError, FsNode, Vfs, MAX_MOUNTS};
use crate::path::strip_mount;
use alloc::string::String;
use core::fmt::{self, Write};

struct Output {
    text: String,
    limit: usize,
}

impl Write for Output {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if text.len() > self.limit - self.text.len() {
            return Err(fmt::Error);
        }
        self.text.push_str(text);
        Ok(())
    }
}

fn escape(output: &mut Output, path: &str) -> fmt::Result {
    for (start, character) in path.char_indices() {
        match character {
            ' ' => output.write_str("\\040")?,
            '\t' => output.write_str("\\011")?,
            '\n' => output.write_str("\\012")?,
            '\\' => output.write_str("\\134")?,
            _ => output.write_str(&path[start..start + character.len_utf8()])?,
        }
    }
    Ok(())
}

impl<D: BlockDevice> Vfs<D> {
    pub fn linux_mountinfo(
        &self,
        limit: usize,
        registered_block: impl Fn(u64) -> bool,
    ) -> Result<String, FsError> {
        let mut mounts = [None; MAX_MOUNTS];
        for (slot, mount) in mounts.iter_mut().zip(self.mounts.iter().flatten()) {
            *slot = Some(mount);
        }
        mounts[..self.count].sort_unstable_by_key(|mount| {
            let mount = mount.unwrap();
            (core::cmp::Reverse(mount.path.len()), mount.tag)
        });
        let mut output = Output {
            text: String::new(),
            limit,
        };
        for mount in mounts.iter().flatten() {
            let id = u64::from(mount.tag) + 1;
            let parent = mounts.iter().flatten().find(|candidate| {
                candidate.path != mount.path && strip_mount(&mount.path, &candidate.path).is_some()
            });
            let parent_id = match parent {
                Some(parent) => u64::from(parent.tag) + 1,
                None if mount.path == "/" => id,
                None => return Err(FsError::Unsupported),
            };
            let (major, minor, fs_type) = match &mount.fs {
                FsNode::Ram(_) => (0, if mount.path == "/" { 0 } else { id }, "tmpfs"),
                FsNode::Fat(_)
                    if (1..(1 << 20)).contains(&mount.source) && registered_block(mount.source) =>
                {
                    (240, mount.source, "vfat")
                }
                FsNode::Fat(_) => return Err(FsError::Unsupported),
            };
            let format_error = |_| FsError::NoMemory;
            write!(output, "{id} {parent_id} {major}:{minor} / ").map_err(format_error)?;
            escape(&mut output, &mount.path).map_err(format_error)?;
            write!(output, " rw - {fs_type} ").map_err(format_error)?;
            if major == 0 {
                output.write_str("tmpfs").map_err(format_error)?;
            } else {
                write!(output, "helios-block:{minor}").map_err(format_error)?;
            }
            output.write_str(" rw\n").map_err(format_error)?;
        }
        Ok(output.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tmpfs;
    use sunlight_block::NullDevice;

    #[test]
    fn linux_fields_and_nearest_parent() {
        let mut vfs = Vfs::<NullDevice>::new();
        for path in ["/", "/mnt", "/mnt/a b\t\n\\é", "/mnt-other"] {
            vfs.mount_tmpfs(path, Tmpfs::new(&[])).unwrap();
        }
        let output = vfs.linux_mountinfo(8192, |_| false).unwrap();
        let fields: alloc::vec::Vec<_> = output
            .lines()
            .map(|line| {
                line.split_ascii_whitespace()
                    .collect::<alloc::vec::Vec<_>>()
            })
            .collect();
        assert_eq!(
            fields[0],
            [
                "3",
                "2",
                "0:3",
                "/",
                "/mnt/a\\040b\\011\\012\\134é",
                "rw",
                "-",
                "tmpfs",
                "tmpfs",
                "rw"
            ]
        );
        assert_eq!((fields[1][1], fields[1][4]), ("1", "/mnt-other"));
        assert_eq!((fields[2][1], fields[2][4]), ("1", "/mnt"));
        assert_eq!((fields[3][1], fields[3][4]), ("1", "/"));
        assert_eq!(
            vfs.linux_mountinfo(output.len() - 1, |_| false),
            Err(FsError::NoMemory)
        );
    }
}
