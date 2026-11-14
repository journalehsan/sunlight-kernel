#!/usr/bin/env python3
"""SFS-0 evidence validation only; no filesystem codec/engine or image writes.

Run from anywhere: python3 docs/audit/sunfs/sfs-0/validate.py
Hashes incoming regular files; checks proposed field arithmetic, local doc links,
required artifacts, CRC convention and bounded geometry model; writes audit JSON.
"""
import hashlib
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
OUT = ROOT / 'docs/audit/sunfs/sfs-0'
DOC = ROOT / 'docs/filesystems/sunfs'


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def save(name, value):
    (OUT / name).write_text(json.dumps(value, indent=2) + '\n')


baseline = json.loads((OUT / 'source-baseline.json').read_text())
changed = []
for name, expected in baseline['sha256'].items():
    path = ROOT / name
    if not path.is_file() or path.is_symlink() or digest(path) != expected:
        changed.append(name)

current_paths = subprocess.check_output(
    ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd=ROOT
).decode().split('\0')
allowed = ('docs/filesystems/sunfs/', 'docs/audit/sunfs/sfs-0/')
extra = [name for name in current_paths if name and name not in baseline['sha256']
         and not name.startswith(allowed) and (ROOT / name).is_file()
         and not (ROOT / name).is_symlink()]

required_docs = [
    'sunfs-architecture-v0.2.md', 'sunfs-on-disk-format-v0.2.md',
    'sunfs-crash-recovery-contract.md', 'sunfs-vfs-integration.md',
    'sunfs-implementation-roadmap.md', 'adr/0001-checkpointed-segment-log.md',
]
required_evidence = [
    'README.md', 'repository-capability-inventory.md', 'existing-draft-comparison.md',
    'block-device-contract-assessment.md', 'architecture-decision-summary.md',
    'format-field-inventory.json', 'geometry-model.json', 'recovery-invariants.md',
    'risk-register.md', 'design-review-checklist.md', 'source-baseline.json',
    'source-anchors.json', 'validate.py',
]
missing = [str(p.relative_to(ROOT)) for p in
           [DOC / name for name in required_docs] +
           [OUT / name for name in required_evidence] if not p.is_file()]

broken_links = []
for path in sorted(DOC.rglob('*.md')) + sorted(OUT.glob('*.md')):
    for target in re.findall(r'\[[^\]]+\]\(([^)]+)\)', path.read_text()):
        if '://' in target or target.startswith('#'):
            continue
        local = target.split('#', 1)[0]
        if not (path.parent / local).resolve().exists():
            broken_links.append({'source': str(path.relative_to(ROOT)), 'target': target})

inventory = json.loads((OUT / 'format-field-inventory.json').read_text())
expected_sizes = {
    'common_header': 96, 'superblock_payload': 136, 'checkpoint_payload': 160,
    'node_id': 16, 'block_ref': 24, 'time': 16, 'node_payload': 200,
    'table_root_prefix': 24, 'page_descriptor': 48, 'table_page_prefix': 24,
    'node_map_entry': 48, 'directory_page_prefix': 32, 'directory_entry_prefix': 32,
    'extent_page_prefix': 24, 'extent': 48, 'segment_state_prefix': 64,
    'summary_page_prefix': 48, 'summary_entry': 48, 'orphan_entry': 32,
}
field_errors = []
for name, size in expected_sizes.items():
    record = inventory['records'][name]
    offset = 0
    for field in record['fields']:
        if field['offset'] != offset or field['bytes'] <= 0:
            field_errors.append(name + ':' + field['name'])
        offset += field['bytes']
    if offset != size or record['fixed_bytes'] != size:
        field_errors.append(name + ':size')
header = inventory['records']['common_header']['fields']
if next(f for f in header if f['name'] == 'crc64')['offset'] != 64:
    field_errors.append('crc_offset')
format_text = (DOC / 'sunfs-on-disk-format-v0.2.md').read_text()
for text in ['SB payload is 136 bytes', 'CP payload is 160 bytes',
             'Node payload is 200 bytes', '24+82*48=3960']:
    if text not in format_text:
        field_errors.append('document:' + text)

crc = 0
for value in b'123456789':
    crc ^= value << 56
    for _ in range(8):
        crc = ((crc << 1) ^ (0x42F0E1EBA9EA3693 if crc & (1 << 63) else 0)) & ((1 << 64) - 1)
crc_ok = crc == 0x6C40DF5F0B497347

model = json.loads((OUT / 'geometry-model.json').read_text())
model_errors = []
for row in model['models']:
    blocks = row['blocks_per_segment']
    bitmap = (blocks + 7) // 8
    record = ((64 + bitmap + 7) // 8) * 8
    maximum = 82 * (3976 // record)
    if (row['segment_bytes'] != blocks * 4096 or
            row['segment_state_record_bytes'] != record or
            row['max_segments_by_single_level_table'] != maximum or
            row['max_summary_pages_at_full_liveness'] != (blocks + 81) // 82 or
            row['union_bitmap_bytes_at_table_cap'] != (maximum * blocks + 7) // 8):
        model_errors.append(blocks)

anchors = json.loads((OUT / 'source-anchors.json').read_text())
anchor_errors = []
for src in anchors['sources']:
    path = ROOT / src['path']
    lines = path.read_text().splitlines()
    if digest(path) != src['sha256'] or baseline['sha256'].get(src['path']) != src['sha256']:
        anchor_errors.append(src['path'] + ':hash')
    for anchor in src['anchors']:
        if not anchor['matches']:
            anchor_errors.append(src['path'] + ':' + anchor['pattern'])
        for hit in anchor['matches']:
            if lines[hit['line'] - 1] != hit['text']:
                anchor_errors.append(src['path'] + ':line')

checks = {
    'evidence_kind': 'documentation_source_integrity_and_arithmetic_only',
    'command': 'python3 docs/audit/sunfs/sfs-0/validate.py',
    'incoming_regular_files_verified': len(baseline['sha256']),
    'changed_or_missing_incoming_files': changed,
    'unexpected_new_files_outside_output_directories': extra,
    'missing_required_files': missing, 'broken_local_document_links': broken_links,
    'format_field_arithmetic_errors': field_errors,
    'crc64_ecma_check_value': f'{crc:016x}', 'crc64_convention_matches': crc_ok,
    'geometry_arithmetic_errors': model_errors,
    'source_anchor_files_verified': len(anchors['sources']),
    'source_anchor_errors': anchor_errors,
    'production_tests': 'NOT_RUN', 'format_serialization_tests': 'NOT_RUN',
    'crash_recovery_tests': 'NOT_RUN', 'driver_durability_tests': 'NOT_RUN',
    'qemu_tests': 'NOT_RUN', 'application_ui_tests': 'NOT_RUN',
}
ok = not any([changed, extra, missing, broken_links, field_errors,
              model_errors, anchor_errors]) and crc_ok
checks['status'] = 'PASS' if ok else 'BLOCKED'
save('validation-checks.json', checks)

criteria = {
    'G1': ('Repository and v0.1 audit', ['repository-capability-inventory.md', 'existing-draft-comparison.md']),
    'G2': ('Selected architecture and alternatives', ['architecture-decision-summary.md', 'docs/filesystems/sunfs/adr/0001-checkpointed-segment-log.md']),
    'G3': ('Stable identity and namespace lifetime', ['docs/filesystems/sunfs/sunfs-architecture-v0.2.md', 'docs/filesystems/sunfs/sunfs-vfs-integration.md']),
    'G4': ('Reviewable format proposal, not frozen layout', ['docs/filesystems/sunfs/sunfs-on-disk-format-v0.2.md', 'format-field-inventory.json']),
    'G5': ('Persistence requirements and missing APIs', ['block-device-contract-assessment.md']),
    'G6': ('Checkpoint, fsync and recovery contract', ['docs/filesystems/sunfs/sunfs-crash-recovery-contract.md', 'recovery-invariants.md']),
    'G7': ('Allocation and GC safety model', ['docs/filesystems/sunfs/sunfs-architecture-v0.2.md', 'risk-register.md']),
    'G8': ('Hosted laboratory and crash plan', ['docs/filesystems/sunfs/sunfs-implementation-roadmap.md']),
    'G9': ('VFS stages and milestone roadmap', ['docs/filesystems/sunfs/sunfs-vfs-integration.md', 'docs/filesystems/sunfs/sunfs-implementation-roadmap.md']),
    'G10': ('Verified source integrity and design evidence', ['source-integrity-manifest.json', 'validation-checks.json', 'design-review-checklist.md']),
}
summary = {
    'milestone': 'SFS-0', 'date': '2026-11-14',
    'overall': 'SFS-0 COMPLETE' if ok else 'BLOCKED',
    'scope': 'Design and source audit only; no production implementation or proven crash safety',
    'gate_basis': 'Static source review and authored design review checklist; mechanical checks validate evidence integrity, not filesystem correctness',
    'evidence_path_rule': 'Paths starting docs/ are repository-relative; other paths are relative to docs/audit/sunfs/sfs-0/',
    'gates': {gate: {'status': 'PASS' if ok else ('BLOCKED' if gate == 'G10' else 'PASS'),
                     'requirement': title, 'evidence': evidence}
              for gate, (title, evidence) in criteria.items()},
    'execution': {name: 'NOT_RUN' for name in ['filesystem_implementation', 'format_serialization', 'persistence_crash_tests', 'existing_filesystem_regressions', 'qemu', 'native_ui', 'real_hardware']},
    'executed_checks': ['source regular-file SHA256 comparison', 'source anchors',
                        'required documents/local links', 'proposed field byte arithmetic',
                        'CRC convention vector', 'segment geometry arithmetic'],
    'critical_missing_native_contracts': ['common geometry/capability query',
        'uncertain/partial I/O provenance', 'timeout quiescence and DMA fencing',
        'driver/platform durability qualification', 'retained directory relative operations',
        'versioned full-width identity/metadata and lossless directory enumeration'],
    'production_changes': [], 'boot_root_migration': False, 'commit': False, 'push': False,
}
save('validation-summary.json', summary)
outputs = {}
for parent in [DOC, OUT]:
    for path in sorted(parent.rglob('*')):
        if path.is_file() and path.name != 'source-integrity-manifest.json':
            outputs[str(path.relative_to(ROOT))] = digest(path)
manifest = {
    'revision': baseline['revision'], 'baseline_manifest_sha256': digest(OUT / 'source-baseline.json'),
    'baseline_scope': baseline['scope'],
    'incoming_regular_files_verified': len(baseline['sha256']),
    'incoming_changed_or_missing': changed,
    'new_files_outside_output_directories': extra,
    'status': 'PASS' if ok else 'BLOCKED',
    'outputs_sha256': outputs,
    'self_hash_excluded': True,
    'limitations': ['No symlink/directory metadata integrity assertion',
                    'Ignored generated artifacts excluded',
                    'No production/runtime/format/crash execution',
                    'Hashes detect byte changes, not design correctness'],
}
save('source-integrity-manifest.json', manifest)
print(json.dumps({'status': checks['status'], 'source_files': len(baseline['sha256']),
                  'sources_unchanged': not changed, 'outputs': len(outputs),
                  'errors': {k: v for k, v in checks.items() if
                             ('errors' in k or k.startswith(('missing', 'broken', 'unexpected', 'changed'))) and v}}, indent=2))
raise SystemExit(0 if ok else 1)
