//! Bounded document-generation census and integrity diagnostics.
//!
//! Used to prove uncertain commits and restarts do not create duplicate
//! active document generations. Operations are paginated and never load an
//! unbounded snapshot beyond the configured page limit.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use wiseowl_memory::SourceId;

use crate::attributes::AttributeValue;
use crate::record::{LongTermMemoryKind, LongTermMemoryRecord, LongTermRecordState};

/// Per-source generation census (document records only).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceGenerationCensus {
    pub source_id: u64,
    pub active_document_generations: u32,
    pub superseded_document_generations: u32,
    pub staged_generations: u32,
    pub active_chunk_records: u32,
    pub orphan_chunk_records: u32,
    pub duplicate_import_keys: u32,
    pub latest_revision: Option<u32>,
}

/// Global database generation census totals.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DatabaseGenerationCensus {
    pub sources: u32,
    pub active_document_generations: u64,
    pub superseded_document_generations: u64,
    pub sources_with_multiple_active_generations: u32,
    pub duplicate_import_keys: u32,
    pub orphan_chunks: u32,
    pub invalid_supersession_chains: u32,
}

/// Structured verify-generations result (no payloads).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GenerationVerifyResult {
    pub ok: bool,
    pub census: DatabaseGenerationCensus,
    pub multi_active_sources: u32,
    pub duplicate_import_keys: u32,
    pub orphan_chunks: u32,
    pub invalid_supersession_chains: u32,
}

fn is_document(rec: &LongTermMemoryRecord) -> bool {
    match rec.attributes.get("record_role") {
        Some(AttributeValue::Text(role)) => role == "document",
        _ => {
            matches!(
                rec.kind,
                LongTermMemoryKind::ImportedRecord | LongTermMemoryKind::Observation
            ) && rec.provenance.source_id.is_some()
                && rec.attributes.get("import_key").is_some()
        }
    }
}

fn is_chunk(rec: &LongTermMemoryRecord) -> bool {
    matches!(
        rec.attributes.get("record_role"),
        Some(AttributeValue::Text(role)) if role == "chunk" || role == "document_chunk"
    )
}

fn source_revision(rec: &LongTermMemoryRecord) -> u32 {
    match rec.attributes.get("source_revision") {
        Some(AttributeValue::Unsigned(v)) => *v as u32,
        _ => rec.revision,
    }
}

fn import_key(rec: &LongTermMemoryRecord) -> Option<&str> {
    match rec.attributes.get("import_key") {
        Some(AttributeValue::Text(s)) if !s.is_empty() => Some(s.as_str()),
        _ => None,
    }
}

/// Compute census over a bounded page of records keyed by source id.
pub fn census_from_records(
    records: &BTreeMap<u64, LongTermMemoryRecord>,
    source_filter: Option<SourceId>,
    max_sources: usize,
) -> (DatabaseGenerationCensus, Vec<SourceGenerationCensus>) {
    let mut by_source: BTreeMap<u64, SourceGenerationCensus> = BTreeMap::new();
    let mut active_doc_ids: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    let mut all_doc_ids: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    let mut import_keys: BTreeMap<(u64, String), u32> = BTreeMap::new();

    for rec in records.values() {
        let Some(sid) = rec.provenance.source_id else {
            continue;
        };
        if let Some(filter) = source_filter {
            if sid != filter {
                continue;
            }
        }
        let entry = by_source
            .entry(sid.get())
            .or_insert_with(|| SourceGenerationCensus {
                source_id: sid.get(),
                ..Default::default()
            });
        if is_document(rec) {
            all_doc_ids.entry(sid.get()).or_default().push(rec.id.get());
            match rec.state {
                LongTermRecordState::Active => {
                    entry.active_document_generations =
                        entry.active_document_generations.saturating_add(1);
                    active_doc_ids
                        .entry(sid.get())
                        .or_default()
                        .push(rec.id.get());
                }
                LongTermRecordState::Superseded => {
                    entry.superseded_document_generations =
                        entry.superseded_document_generations.saturating_add(1);
                }
                _ => {}
            }
            let rev = source_revision(rec);
            entry.latest_revision = Some(match entry.latest_revision {
                Some(prev) => prev.max(rev),
                None => rev,
            });
            if let Some(key) = import_key(rec) {
                if rec.state == LongTermRecordState::Active {
                    *import_keys.entry((sid.get(), key.to_string())).or_default() += 1;
                }
            }
        } else if is_chunk(rec) {
            match rec.state {
                LongTermRecordState::Active => {
                    entry.active_chunk_records = entry.active_chunk_records.saturating_add(1);
                }
                _ => {}
            }
        }
    }

    // Orphan active chunks: active chunk whose source has no document record.
    for entry in by_source.values_mut() {
        let has_doc = all_doc_ids
            .get(&entry.source_id)
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        if entry.active_chunk_records > 0 && !has_doc {
            entry.orphan_chunk_records = entry.active_chunk_records;
        }
    }

    // Duplicate active import keys per source.
    for ((sid, _), count) in &import_keys {
        if *count > 1 {
            if let Some(entry) = by_source.get_mut(sid) {
                entry.duplicate_import_keys = entry
                    .duplicate_import_keys
                    .saturating_add(count.saturating_sub(1));
            }
        }
    }

    // Supersession chain validation: detect simple loops (A supersedes B, B supersedes A).
    let mut invalid_chains = 0u32;
    for rec in records.values() {
        if !is_document(rec) {
            continue;
        }
        if let Some(old) = rec.supersedes {
            if let Some(prev) = records.get(&old.get()) {
                if prev.supersedes == Some(rec.id) {
                    invalid_chains = invalid_chains.saturating_add(1);
                }
            }
        }
    }

    let mut per_source: Vec<SourceGenerationCensus> = by_source.into_values().collect();
    per_source.sort_by_key(|s| s.source_id);
    if per_source.len() > max_sources {
        per_source.truncate(max_sources);
    }

    let mut global = DatabaseGenerationCensus::default();
    global.sources = per_source.len() as u32;
    for s in &per_source {
        global.active_document_generations = global
            .active_document_generations
            .saturating_add(s.active_document_generations as u64);
        global.superseded_document_generations = global
            .superseded_document_generations
            .saturating_add(s.superseded_document_generations as u64);
        if s.active_document_generations > 1 {
            global.sources_with_multiple_active_generations = global
                .sources_with_multiple_active_generations
                .saturating_add(1);
        }
        global.duplicate_import_keys = global
            .duplicate_import_keys
            .saturating_add(s.duplicate_import_keys);
        global.orphan_chunks = global.orphan_chunks.saturating_add(s.orphan_chunk_records);
    }
    global.invalid_supersession_chains = invalid_chains;

    (global, per_source)
}

pub fn verify_generations(
    records: &BTreeMap<u64, LongTermMemoryRecord>,
    max_sources: usize,
) -> GenerationVerifyResult {
    let (census, _) = census_from_records(records, None, max_sources);
    let ok = census.sources_with_multiple_active_generations == 0
        && census.duplicate_import_keys == 0
        && census.orphan_chunks == 0
        && census.invalid_supersession_chains == 0;
    GenerationVerifyResult {
        ok,
        multi_active_sources: census.sources_with_multiple_active_generations,
        duplicate_import_keys: census.duplicate_import_keys,
        orphan_chunks: census.orphan_chunks,
        invalid_supersession_chains: census.invalid_supersession_chains,
        census,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::{Attribute, AttributeSet, AttributeValue};
    use crate::provenance::{DerivationKind, LongTermProvenance};
    use crate::record::{LongTermMemoryKind, LongTermRecordState, MemoryScope, PayloadRef};
    use wiseowl_memory::{MemoryId, SourceId, SourceKind, TrustLevel};

    fn doc(
        id: u64,
        sid: u64,
        rev: u32,
        state: LongTermRecordState,
        key: &str,
    ) -> LongTermMemoryRecord {
        let mut attributes = AttributeSet {
            entries: alloc::vec![
                Attribute {
                    key: String::from("record_role"),
                    value: AttributeValue::Text(String::from("document")),
                },
                Attribute {
                    key: String::from("source_revision"),
                    value: AttributeValue::Unsigned(rev as u64),
                },
                Attribute {
                    key: String::from("import_key"),
                    value: AttributeValue::Text(String::from(key)),
                },
            ],
        };
        attributes.normalize();
        LongTermMemoryRecord {
            format_version: 1,
            id: MemoryId::from_raw_unchecked(id),
            revision: rev,
            kind: LongTermMemoryKind::ImportedRecord,
            scope: MemoryScope::User,
            owner: 1,
            created_at_ns: 1,
            updated_at_ns: 1,
            valid_from_ns: None,
            valid_until_ns: None,
            importance: 0,
            confidence: 0,
            trust: TrustLevel::Untrusted,
            provenance: LongTermProvenance {
                source_kind: SourceKind::UserInput,
                source_id: Some(SourceId::from_raw_unchecked(sid)),
                producer_service: String::from("test"),
                original_memory_ids: Vec::new(),
                parent_lt_ids: Vec::new(),
                insertion_time_ns: 1,
                trust: TrustLevel::Untrusted,
                source_content_hash: None,
                external_ref: None,
                derivation: DerivationKind::DirectImport,
            },
            payload_ref: PayloadRef {
                content_hash: 0,
                length: 0,
            },
            tokens: None,
            attributes,
            state,
            supersedes: None,
            payload: Vec::new(),
            token_entries: Vec::new(),
        }
    }

    #[test]
    fn single_active_generation_ok() {
        let mut records = BTreeMap::new();
        records.insert(1, doc(1, 10, 1, LongTermRecordState::Superseded, "k1"));
        records.insert(2, doc(2, 10, 2, LongTermRecordState::Active, "k2"));
        let (g, per) = census_from_records(&records, None, 64);
        assert_eq!(g.sources, 1);
        assert_eq!(g.active_document_generations, 1);
        assert_eq!(g.superseded_document_generations, 1);
        assert_eq!(g.sources_with_multiple_active_generations, 0);
        assert_eq!(per[0].latest_revision, Some(2));
        assert!(verify_generations(&records, 64).ok);
    }

    #[test]
    fn duplicate_active_detected() {
        let mut records = BTreeMap::new();
        records.insert(1, doc(1, 10, 1, LongTermRecordState::Active, "k1"));
        records.insert(2, doc(2, 10, 2, LongTermRecordState::Active, "k2"));
        let v = verify_generations(&records, 64);
        assert!(!v.ok);
        assert_eq!(v.multi_active_sources, 1);
    }
}
