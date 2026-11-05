#!/usr/bin/env python3
"""Focused R1B host validation. Never substitutes host checks for a native pilot."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / '.fastlane-evidence/m61b-r1b'
CONTROLS = (
    'forged-caller', 'unauthorized-operation', 'stale-policy-generation',
    'wrong-channel-generation', 'duplicate-publication', 'required-downgrade',
    'kernel-only-fastlane', 'cancel-before-commit', 'peer-exit-incomplete',
    'fallback-after-publication',
)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source(out, name):
    paths = subprocess.check_output(
        ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd=ROOT
    ).decode().split('\0')
    manifest = {p: sha(ROOT / p) for p in sorted(set(paths))
                if p and not p.startswith('.fastlane-evidence/') and (ROOT / p).is_file()}
    dest = out / name
    dest.write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
    return sha(dest)


def main():
    out = BASE / datetime.now(timezone.utc).strftime('validation-%Y%m%dT%H%M%S.%fZ')
    out.mkdir(parents=True)
    summary = {'status': 'PARTIAL', 'production_fastlane': 'DISABLED', 'checks': [],
               'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
               'source_before_sha256': source(out, 'source-before.json')}
    (out / 'status-before.txt').write_bytes(subprocess.check_output(['git', 'status', '--porcelain=v1'], cwd=ROOT))
    (out / 'tree-before.patch').write_bytes(subprocess.check_output(['git', 'diff', '--binary', 'HEAD'], cwd=ROOT))

    def save():
        (out / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')

    def check(name, command, timeout=600):
        record = {'name': name, 'command': list(map(str, command)), 'timeout_seconds': timeout}
        summary['checks'].append(record)
        save()
        with (out / (name + '.log')).open('w') as log:
            log.write(json.dumps(record) + '\n'); log.flush()
            try:
                result = subprocess.run(record['command'], cwd=ROOT, env=os.environ.copy(),
                                        stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
                record['exit_code'] = result.returncode
            except subprocess.TimeoutExpired:
                record['exit_code'] = 'TIMEOUT'
        save()
        print(name, record['exit_code'], flush=True)
        if record['exit_code'] != 0:
            raise RuntimeError(name + ' failed; later stages NOT_RUN')

    print(out, flush=True)
    save()
    try:
        binary = out / 'operation-tests'
        check('operation-compile', ['rustc', '--edition', '2021', '--test',
                                  'ipc/src/fastlane_operation.rs', '-o', binary])
        check('operation-equivalence', [binary, 'operation_equivalence', '--nocapture'])
        check('operation-layout', [binary, 'independent_wire_encodings', '--nocapture'])
        check('operation-terminal', [binary, 'failure_is_terminal', '--nocapture'])
        host = ['cargo', 'test', '--locked', '--offline', '--target', 'x86_64-unknown-linux-gnu']
        check('ipc-envelope', host + ['-p', 'sunlight-ipc', '--lib',
                                     'fastlane_operation_ipc::tests::exact_ipc_envelope'])
        check('kernel-policy-contract', host + ['-p', 'sunlight-kernel', '--lib',
                                               'capability::channel_contract::tests', '--', '--test-threads=1'])
        check('operation-parser-negatives', [binary, 'parser_negative_controls', '--nocapture'])
        check('ipc-parser-negatives', host + ['-p', 'sunlight-ipc', '--lib',
                                            'fastlane_operation_ipc::tests::malformed_envelope'])
        check('experimental-ipc-check', ['cargo', 'check', '--locked', '--offline', '-p',
                                         'sunlight-ipc', '--features', 'experimental-fastlane-authority'])
        check('experimental-check', ['cargo', 'check', '--locked', '--offline', '-p',
                                     'sunlight-kernel', '--features', 'fastlane_contract_test'])
        summary['native_2cpu'] = {
            'status': 'NOT_RUN', 'requests': 0, 'responses': 0,
            'reason': 'No authenticated R1B operation dispatch or source-matched pilot image exists. '
                      'Running an M5.2 workload cannot prove this operation.',
            'controls': {control: 'NOT_RUN' for control in CONTROLS},
            'retirement': 'NOT_RUN', 'remote_tlb_ack': 'NOT_RUN',
        }
        save()
        check('retirement-parser-regressions', ['python3', '-m', 'unittest', 'discover',
                                               '-s', 'tools', '-p', 'test_fastlane_mapping.py'])
        check('ordinary-release', ['cargo', 'build', '--locked', '--offline', '--release',
                                   '-p', 'sunlight-kernel'])
        elf = ROOT / 'target/x86_64-unknown-none/release/sunlight-kernel'
        markers = [b'[FL-M61B-K]', b'/test/contract-audio', b'policy_generation=',
                   b'fastlane_operation', b'[FL-NATIVE-RING]']
        data = elf.read_bytes()
        summary['release'] = {'elf': str(elf), 'sha256': sha(elf),
                              'markers': {m.decode(): m in data for m in markers}}
        if any(summary['release']['markers'].values()):
            raise RuntimeError('experimental release marker found')
        check('whitespace', ['git', 'diff', '--check'])
    except Exception as error:
        summary['failure'] = str(error)
    finally:
        summary['source_after_sha256'] = source(out, 'source-after.json')
        summary['source_unchanged_during_validation'] = (
            summary['source_before_sha256'] == summary['source_after_sha256'])
        save()
    # Partial native proof is an explicit unsuccessful acceptance result.
    return 1


if __name__ == '__main__':
    raise SystemExit(main())
