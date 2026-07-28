use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::vec::Vec;

use wiseowl_index::quotas::IndexQuotaConfig;
use wiseowl_index::tokenize::{NormalizedTextBuffer, TokenDictionary, TokenSink};
use wiseowl_index::{RetrievalTokenizer, WiseOwlLexicalV1};

const FOUNDATION_FORMAT_VERSION: u16 = 1;
const FOUNDATION_SCHEMA_VERSION: u16 = 1;
const FOUNDATION_MAGIC: &[u8; 8] = b"WOFM\x01\0\0\0";
const HEADER_LEN: usize = 106;
const HASH_OFFSET: usize = 74;

const FOUNDATION_KEYS: &[(u16, &str)] = &[
    (1, "assistant_name"),
    (2, "internal_codename"),
    (3, "sunlightos_identity"),
    (4, "general_role"),
    (5, "high_level_capabilities"),
    (6, "safety_principles"),
    (7, "capability_security_model"),
    (8, "runtime_info_guidance"),
    (9, "memory_layer_model"),
];

fn main() {
    if let Err(error) = build_foundation_blob() {
        panic!("failed to build Wise Owl foundation blob: {error}");
    }
}

fn build_foundation_blob() -> Result<(), String> {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").map_err(|err| err.to_string())?);
    let workspace_root = manifest_dir
        .parent()
        .ok_or_else(|| "wiseowl-brain has no workspace parent".to_string())?
        .to_path_buf();
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").map_err(|err| err.to_string())?);
    let foundation_source = manifest_dir.join("foundation/foundation_v1.txt");

    println!("cargo:rerun-if-changed={}", foundation_source.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("build.rs").display()
    );

    let tokenizer_files = [
        workspace_root.join("wiseowl-index/src/config.rs"),
        workspace_root.join("wiseowl-index/src/tokenize/mod.rs"),
        workspace_root.join("wiseowl-index/src/tokenize/lexical.rs"),
        workspace_root.join("wiseowl-index/src/tokenize/normalize.rs"),
        workspace_root.join("wiseowl-index/src/tokenize/dictionary.rs"),
    ];
    for path in &tokenizer_files {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let source = fs::read_to_string(&foundation_source)
        .map_err(|err| format!("read {}: {err}", foundation_source.display()))?;
    let records = parse_foundation_source(&source)?;

    let tokenizer = WiseOwlLexicalV1::default();
    let quotas = IndexQuotaConfig::default();
    let tokenizer_id = tokenizer.tokenizer_id();
    let tokenizer_version = tokenizer.version();
    let tokenizer_fingerprint = tokenizer_fingerprint(&tokenizer_files)?;

    let mut compiled_records = Vec::with_capacity(records.len());
    let mut compiled_tokens = Vec::new();

    for (key_tag, key_name, value) in records {
        let token_offset = compiled_tokens.len() as u32;
        let token_input = format!("{key_name}: {value}");
        let mut normalized = NormalizedTextBuffer::default();
        tokenizer
            .normalize(&token_input, &mut normalized)
            .map_err(|err| format!("normalize {key_name}: {err}"))?;
        let mut dict = TokenDictionary::new();
        let mut sink = TokenSink::default();
        tokenizer
            .tokenize(&normalized.text, &mut dict, &quotas, &mut sink)
            .map_err(|err| format!("tokenize {key_name}: {err}"))?;
        let token_count = sink.tokens.len() as u16;
        for token in sink.tokens {
            compiled_tokens.push(CompiledToken {
                token_id: token.token_id,
                frequency: token.frequency,
                positions_truncated: token.positions_truncated,
                positions: token.positions,
            });
        }
        compiled_records.push(CompiledRecord {
            key_tag,
            value,
            token_offset,
            token_count,
        });
    }

    let records_buf = encode_records(&compiled_records);
    let tokens_buf = encode_tokens(&compiled_tokens);
    let blob = encode_blob(
        tokenizer_id,
        tokenizer_version,
        tokenizer_fingerprint,
        &records_buf,
        compiled_records.len() as u16,
        &tokens_buf,
        compiled_tokens.len() as u32,
    );

    fs::write(out_dir.join("wiseowl-foundation.bin"), &blob)
        .map_err(|err| format!("write wiseowl-foundation.bin: {err}"))?;
    fs::write(
        out_dir.join("foundation_build.rs"),
        render_build_constants(tokenizer_id, tokenizer_version, tokenizer_fingerprint),
    )
    .map_err(|err| format!("write foundation_build.rs: {err}"))?;

    Ok(())
}

#[derive(Debug)]
struct CompiledRecord {
    key_tag: u16,
    value: String,
    token_offset: u32,
    token_count: u16,
}

#[derive(Debug)]
struct CompiledToken {
    token_id: u64,
    frequency: u16,
    positions_truncated: bool,
    positions: Vec<u32>,
}

fn parse_foundation_source(source: &str) -> Result<Vec<(u16, &'static str, String)>, String> {
    let mut map = BTreeMap::<String, String>::new();
    for (lineno, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {}: expected key = value", lineno + 1));
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            return Err(format!("line {}: empty key or value", lineno + 1));
        }
        if map.insert(key.to_string(), value.to_string()).is_some() {
            return Err(format!("duplicate key: {key}"));
        }
    }

    let mut records = Vec::with_capacity(FOUNDATION_KEYS.len());
    for (tag, key) in FOUNDATION_KEYS {
        let Some(value) = map.remove(*key) else {
            return Err(format!("missing required foundation key: {key}"));
        };
        records.push((*tag, *key, value));
    }
    if let Some(extra) = map.keys().next() {
        return Err(format!("unknown foundation key: {extra}"));
    }
    Ok(records)
}

fn tokenizer_fingerprint(paths: &[PathBuf]) -> Result<[u8; 32], String> {
    let mut bytes = Vec::new();
    for path in paths {
        append_fingerprint_file(&mut bytes, path)?;
    }
    Ok(wiseowl_index::digest::sha256(&bytes))
}

fn append_fingerprint_file(out: &mut Vec<u8>, path: &Path) -> Result<(), String> {
    let rel = path.to_string_lossy();
    out.extend_from_slice(rel.as_bytes());
    out.push(0);
    let data = fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    Ok(())
}

fn encode_records(records: &[CompiledRecord]) -> Vec<u8> {
    let mut out = Vec::new();
    for record in records {
        out.extend_from_slice(&record.key_tag.to_le_bytes());
        out.extend_from_slice(&(record.value.len() as u16).to_le_bytes());
        out.extend_from_slice(&record.token_offset.to_le_bytes());
        out.extend_from_slice(&record.token_count.to_le_bytes());
        out.extend_from_slice(record.value.as_bytes());
    }
    out
}

fn encode_tokens(tokens: &[CompiledToken]) -> Vec<u8> {
    let mut out = Vec::new();
    for token in tokens {
        out.extend_from_slice(&token.token_id.to_le_bytes());
        out.extend_from_slice(&token.frequency.to_le_bytes());
        let flags = if token.positions_truncated {
            1u16
        } else {
            0u16
        };
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&(token.positions.len() as u16).to_le_bytes());
        for position in &token.positions {
            out.extend_from_slice(&position.to_le_bytes());
        }
    }
    out
}

fn encode_blob(
    tokenizer_id: u32,
    tokenizer_version: u32,
    tokenizer_fingerprint: [u8; 32],
    records_buf: &[u8],
    record_count: u16,
    tokens_buf: &[u8],
    token_count: u32,
) -> Vec<u8> {
    let records_offset = HEADER_LEN as u32;
    let records_len = records_buf.len() as u32;
    let tokens_offset = records_offset + records_len;
    let tokens_len = tokens_buf.len() as u32;

    let mut blob = Vec::with_capacity(HEADER_LEN + records_buf.len() + tokens_buf.len());
    blob.extend_from_slice(FOUNDATION_MAGIC);
    blob.extend_from_slice(&FOUNDATION_FORMAT_VERSION.to_le_bytes());
    blob.extend_from_slice(&FOUNDATION_SCHEMA_VERSION.to_le_bytes());
    blob.extend_from_slice(&tokenizer_id.to_le_bytes());
    blob.extend_from_slice(&tokenizer_version.to_le_bytes());
    blob.extend_from_slice(&tokenizer_fingerprint);
    blob.extend_from_slice(&record_count.to_le_bytes());
    blob.extend_from_slice(&token_count.to_le_bytes());
    blob.extend_from_slice(&records_offset.to_le_bytes());
    blob.extend_from_slice(&records_len.to_le_bytes());
    blob.extend_from_slice(&tokens_offset.to_le_bytes());
    blob.extend_from_slice(&tokens_len.to_le_bytes());
    blob.extend_from_slice(&[0u8; 32]);
    blob.extend_from_slice(records_buf);
    blob.extend_from_slice(tokens_buf);

    let mut hashed = Vec::with_capacity(blob.len().saturating_sub(32));
    hashed.extend_from_slice(&blob[..HASH_OFFSET]);
    hashed.extend_from_slice(&blob[HASH_OFFSET + 32..]);
    let integrity_hash = wiseowl_index::digest::sha256(&hashed);
    blob[HASH_OFFSET..HASH_OFFSET + 32].copy_from_slice(&integrity_hash);
    blob
}

fn render_build_constants(
    tokenizer_id: u32,
    tokenizer_version: u32,
    tokenizer_fingerprint: [u8; 32],
) -> String {
    let bytes = tokenizer_fingerprint
        .iter()
        .map(|byte| byte.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "pub const FOUNDATION_EXPECTED_TOKENIZER_ID: u32 = {tokenizer_id};\n\
pub const FOUNDATION_EXPECTED_TOKENIZER_VERSION: u32 = {tokenizer_version};\n\
pub const FOUNDATION_EXPECTED_TOKENIZER_FINGERPRINT: [u8; 32] = [{bytes}];\n"
    )
}
