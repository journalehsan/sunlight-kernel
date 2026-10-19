#!/usr/bin/env bash
# Crash-safe ownership slice: blocked successors, not general adoption/recovery.
set -euo pipefail
cd "$(dirname "$0")/.."
scenario="${D36R_SCENARIO:-after-ready}"
evidence="target/d36r-evidence/$scenario"
mkdir -p "$evidence"
python3 - "$evidence" "$scenario" <<'PY'
from pathlib import Path
import hashlib,json,subprocess,sys,datetime
root=Path(sys.argv[1])
paths=subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard'],text=True).splitlines()
(root/'source-sha256.json').write_text(json.dumps({p:hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in paths if Path(p).is_file() and p.endswith(('.rs','.toml','.py','.sh','.ld'))},indent=2)+'\n')
(root/'context.json').write_text(json.dumps({'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'scenario':sys.argv[2],'cpus':sys.argv[3:] or 'gate arguments below','rustc':subprocess.check_output(['rustc','-vV'],text=True),'qemu':subprocess.check_output(['qemu-system-x86_64','--version'],text=True),'outcome':'RecoveryBlocked; no adoption or retirement authority transferred'},indent=2)+'\n')
PY
D36R_SCENARIO="$scenario" RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build -p sunlight-init -p sunlightd -p sunlight-startup-fixture --features sunlight-init/recovery-test,sunlightd/recovery-test,sunlight-startup-fixture/recovery-test --release > "$evidence/userspace-build.log" 2>&1
cargo build -p sunlight-kernel --features recovery_test > "$evidence/kernel-build.log" 2>&1
tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel target/sunlightd-d36r.iso > "$evidence/iso-build.log" 2>&1
sha256sum target/sunlightd-d36r.iso > "$evidence/iso-sha256.txt"
python3 tools/sunlightd_d1_gate.py --d2 --d3 --d36r --d36r-scenario "$scenario" --timeout 300 \
  --iso target/sunlightd-d36r.iso --evidence "$evidence" "$@"
