use std::{collections::HashMap, env, fs};
use sunlight_fs::{
    boot_resources::{checksum, entries, BLOCK, DESCRIPTOR, ENTRY, HEADER, MAGIC, MAX_BYTES},
    INITRAMFS,
};

fn put(output: &mut Vec<u8>, value: usize) {
    output.extend_from_slice(&u32::try_from(value).unwrap().to_le_bytes());
}

fn main() {
    let path = env::args_os().nth(1).expect("output path");
    let mut inventory = Vec::new();
    let mut descriptors = Vec::new();
    let mut payload = Vec::new();
    let inventory_end = HEADER
        + INITRAMFS
            .iter()
            .map(|entry| ENTRY + entry.path.len())
            .sum::<usize>();
    let mut unique = HashMap::new();
    for entry in INITRAMFS {
        let data = entry.data;
        let start = if data.is_empty() {
            0
        } else {
            *unique.entry(data).or_insert_with(|| {
                let start = inventory_end + descriptors.len();
                for block in data.chunks(BLOCK) {
                    let compressed = lz4_flex::block::compress(block);
                    let stored = if compressed.len() < block.len() {
                        &compressed
                    } else {
                        block
                    };
                    put(&mut descriptors, payload.len());
                    put(&mut descriptors, stored.len());
                    put(&mut descriptors, checksum(block) as usize);
                    payload.extend_from_slice(stored);
                }
                start
            })
        };
        put(&mut inventory, entry.path.len());
        put(&mut inventory, entry.uid as usize);
        put(&mut inventory, entry.gid as usize);
        inventory.extend_from_slice(&entry.mode.to_le_bytes());
        inventory.extend_from_slice(&[u8::from(entry.is_dir), 0]);
        put(&mut inventory, data.len());
        put(&mut inventory, start);
        inventory.extend_from_slice(entry.path.as_bytes());
    }
    let payload_start = inventory_end + descriptors.len();
    for desc in descriptors.chunks_exact_mut(DESCRIPTOR) {
        let relative = u32::from_le_bytes(desc[..4].try_into().unwrap()) as usize;
        desc[..4].copy_from_slice(
            &u32::try_from(payload_start + relative)
                .unwrap()
                .to_le_bytes(),
        );
    }
    let size = payload_start + payload.len();
    assert!(size <= MAX_BYTES);
    let mut archive = Vec::with_capacity(size);
    archive.extend_from_slice(MAGIC);
    put(&mut archive, size);
    put(&mut archive, INITRAMFS.len());
    put(&mut archive, payload_start);
    put(&mut archive, inventory_end);
    archive.extend(inventory);
    archive.extend(descriptors);
    archive.extend(payload);
    let archive: &'static [u8] = Box::leak(archive.into_boxed_slice());
    let decoded = entries(archive).expect("generated inventory failed validation");
    assert_eq!(decoded.len(), INITRAMFS.len());
    let mut buffer = [0; 2 * BLOCK + 1];
    for (actual, expected) in decoded.iter().zip(INITRAMFS) {
        assert_eq!(
            (
                actual.path,
                actual.uid,
                actual.gid,
                actual.mode,
                actual.is_dir,
                actual.len()
            ),
            (
                expected.path,
                expected.uid,
                expected.gid,
                expected.mode,
                expected.is_dir,
                expected.data.len()
            )
        );
        for (index, slice) in expected.data.chunks(buffer.len()).enumerate() {
            let count = actual.read(index * buffer.len(), &mut buffer).unwrap();
            assert_eq!(&buffer[..count], slice, "{}", expected.path);
        }
        assert_eq!(actual.read(expected.len(), &mut buffer).unwrap(), 0);
    }
    fs::write(path, archive).expect("write validated boot resource module");
    println!(
        "validated {} entries; {} unique nonempty payloads; {} bytes",
        decoded.len(),
        unique.len(),
        size
    );
}
