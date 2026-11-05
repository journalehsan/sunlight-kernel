"""Reassess all pilot receipts with the final strict parser and source map."""
import hashlib
import json
from pathlib import Path
import sys
sys.path.insert(0, 'tools')
from threading_t2r_receipt import assess_pilot
from threading_t2_validate import identity

base = Path('.threading-evidence/t2r')
current = identity()
inputs = {p: h for p, h in current['files'].items()
          if p.startswith('kernel/') or p.startswith('tools/helios-probes/')
          or p in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', '.cargo/config.toml')}
rows = []
for folder in sorted(base.iterdir()):
    if not (folder / 'result.json').is_file() or not (folder / 'serial.log').is_file():
        continue
    result = json.loads((folder / 'result.json').read_text())
    markers = result.get('markers', [])
    if len(markers) != 1 or not markers[0].startswith('[T2R] PILOT PASS case='):
        continue
    case = int(markers[0].split('case=')[1].split()[0])
    serial = (folder / 'serial.log').read_text()
    source = json.loads((folder / 'source.json').read_text())
    differences = [p for p, h in inputs.items() if source['files'].get(p) != h]
    digest = hashlib.sha256(serial.encode()).hexdigest()
    strict = assess_pilot(serial, case, result['cpus'])
    rows.append({'attempt': folder.name, 'case': case, 'cpus': result['cpus'],
                 'recorded_pass': result['pass'], 'strict_parser_pass': strict,
                 'compiled_inputs_differences': differences,
                 'serial_hash_matches_record': digest == result['serial_sha256'],
                 'serial_sha256': digest, 'iso_sha256': result['iso_sha256'],
                 'eligible_final_pilot': strict and not differences and
                 digest == result['serial_sha256'] and not result.get('timeout'),
                 'winner': next((line.split('actor=')[1] for line in serial.splitlines()
                                 if line.startswith('[T2R] requested-winner')), None)})
(base / 'pilot-receipt-audit.json').write_text(json.dumps(rows, indent=2) + '\n')
print(json.dumps([r for r in rows if r['eligible_final_pilot']], indent=2))
