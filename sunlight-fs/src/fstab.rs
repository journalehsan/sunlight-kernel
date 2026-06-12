pub const MAX_FSTAB_ENTRIES: usize = 8;

#[derive(Debug, Clone, Copy)]
pub struct FstabEntry<'a> {
    pub device: &'a str,
    pub mountpoint: &'a str,
    pub fs_type: &'a str,
    pub options: &'a str,
}

pub type FstabTable<'a> = [Option<FstabEntry<'a>>; MAX_FSTAB_ENTRIES];

pub fn parse_fstab(text: &str) -> FstabTable<'_> {
    let mut table = [None; MAX_FSTAB_ENTRIES];
    let mut count = 0usize;

    for line in text.lines() {
        if count >= MAX_FSTAB_ENTRIES {
            break;
        }

        let content = line
            .split_once('#')
            .map_or(line, |(before_comment, _)| before_comment);
        let mut fields = content.split_whitespace();

        let Some(device) = fields.next() else {
            continue;
        };
        let Some(mountpoint) = fields.next() else {
            continue;
        };
        let Some(fs_type) = fields.next() else {
            continue;
        };
        let Some(options) = fields.next() else {
            continue;
        };

        table[count] = Some(FstabEntry {
            device,
            mountpoint,
            fs_type,
            options,
        });
        count += 1;
    }

    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_whitespace_and_trailing_newline() {
        let table = parse_fstab(
            r#"
# device    mountpoint   type         options
/dev/sda1   /boot        bootfs       defaults

   /dev/ram0      /      ramfs       defaults     # root volume
"#,
        );

        assert_eq!(table[0].unwrap().device, "/dev/sda1");
        assert_eq!(table[0].unwrap().mountpoint, "/boot");
        assert_eq!(table[0].unwrap().fs_type, "bootfs");
        assert_eq!(table[0].unwrap().options, "defaults");
        assert_eq!(table[1].unwrap().device, "/dev/ram0");
        assert_eq!(table[1].unwrap().mountpoint, "/");
        assert_eq!(table[1].unwrap().fs_type, "ramfs");
        assert!(table[2].is_none());
    }

    #[test]
    fn skips_incomplete_lines_without_panicking() {
        let table = parse_fstab(
            r#"
/dev/missing
/dev/sda1 /boot bootfs
/dev/ram0 / ramfs defaults
"#,
        );

        assert_eq!(table[0].unwrap().device, "/dev/ram0");
        assert_eq!(table[0].unwrap().mountpoint, "/");
        assert_eq!(table[1].is_none(), true);
    }

    #[test]
    fn ignores_entries_after_fixed_capacity() {
        let table = parse_fstab(
            r#"
/dev/0 /mnt/0 ramfs defaults
/dev/1 /mnt/1 ramfs defaults
/dev/2 /mnt/2 ramfs defaults
/dev/3 /mnt/3 ramfs defaults
/dev/4 /mnt/4 ramfs defaults
/dev/5 /mnt/5 ramfs defaults
/dev/6 /mnt/6 ramfs defaults
/dev/7 /mnt/7 ramfs defaults
/dev/8 /mnt/8 ramfs defaults
"#,
        );

        assert_eq!(table[0].unwrap().device, "/dev/0");
        assert_eq!(table[MAX_FSTAB_ENTRIES - 1].unwrap().device, "/dev/7");
    }
}
