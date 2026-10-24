#!/usr/bin/env python3
"""E2 final-source evidence. Isolated witnesses never certify service races."""
import argparse
from datetime import datetime, timezone
import gzip
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

from fastlane_f11b0e_validate import ROOT, digest, sources, assess_races, assess_authority_fallback
from fastlane_f11b0c_validate import SERVICE_FLAGS, assess_native
from fastlane_f13nr2v_validate import validate_native as validate_pin
from test_native_doorbell import validate_native as validate_doorbell

BASE = ROOT / 'target/fastlane-f11b0e2-evidence'
BUILD = Path('/tmp/f11b0e2-build')
ENV = {'CARGO_TARGET_DIR': str(BUILD), 'CARGO_INCREMENTAL': '0', 'CARGO_PROFILE_DEV_DEBUG': '0'}


def assess_session(raw, cpus):
    import re
    rows = [dict(re.findall(r'(\w+)=(\S+)', line)) for line in raw.splitlines()
            if line.startswith('[SESSION-E2] sequential ')]
    boundary = [dict(re.findall(r'(\w+)=(\S+)', line)) for line in raw.splitlines()
                if line.startswith('[SESSION-E2] boundary PASS ')]
    errors = []
    if len(rows) != 4 or {r.get('case') for r in rows} != {'0', '1', '2', '3'}:
        errors.append('missing sequential witnesses')
    if len(boundary) != 1:
        errors.append('missing resource and exhaustion boundary')
    for row in rows + boundary:
        if row.get('cpu') != '0' or int(row.get('cpu', '-1')) >= cpus:
            errors.append('invalid CPU identity')
        if row.get('scope') != 'isolated-production-broker' or row.get('live') != '0':
            errors.append('invalid scope or resource count')
    if boundary:
        for k, v in {'registrations':'4','revoked':'2','expired':'1','owner_retired':'1',
                     'receipts':'0','grants':'0','pending_commit':'0','consumption_refs':'0',
                     'revocation_ops':'0','ipc_completions':'0','exhausted':'denied','old_epoch':'denied'}.items():
            if boundary[0].get(k) != v:
                errors.append(f'{k} mismatch')
    if 'PANIC' in raw or 'panicked at' in raw:
        errors.append('guest failure')
    return {'classification': 'FORCED_AND_FAILED' if errors else 'FORCED_AND_PASSED',
            'scope': 'sequential isolated production primitives with synthetic identities',
            'errors': errors, 'checkpoints': rows, 'accounting': boundary,
            'native_session_cases': {f'N{i}': 'NOT_FORCED' for i in range(1, 8)},
            'N8': 'FORCED_AND_FAILED' if errors else 'FORCED_AND_PASSED',
            'trusted_service_authentication': 'NOT_TESTED', 'session_authority': 'PARTIAL'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--section', choices=['checks','production','authority','core','lifecycle','network','all'], default='checks')
    args = parser.parse_args()
    out = BASE / (args.section + '-' + datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'))
    out.mkdir(parents=True)
    summary = {'phase':'F1.1B-0E2', 'section':args.section, 'source_sha256':sources(),
               'checks':[], 'images':{}, 'native':[],
               'gates':{'A':'VERIFIED NATIVE','B':'PARTIAL','session_authority':'PARTIAL',
                        'token_history':'PARTIAL','F1.1B-0':'BLOCKED','C':'BLOCKED'}}
    def save():
        (out/'validation-summary.json').write_text(json.dumps(summary, indent=2)+'\n')
    def check(name, command, env=None, timeout=1200):
        settings = {**ENV, **(env or {})}
        start = time.monotonic()
        with (out/(name+'.log')).open('w') as log:
            log.write(json.dumps({'command':command,'environment':settings})+'\n')
            log.flush()
            try:
                code = subprocess.run(command, cwd=ROOT, env={**os.environ,**settings}, stdout=log,
                                      stderr=subprocess.STDOUT, timeout=timeout).returncode
            except subprocess.TimeoutExpired:
                code = 124
        summary['checks'].append({'name':name,'command':command,'environment':settings,'exit_code':code,
            'classification':'PASS' if code == 0 else 'FAIL','seconds':time.monotonic()-start,
            'log':str((out/(name+'.log')).relative_to(ROOT))})
        print(name, summary['checks'][-1]['classification'], flush=True)
        save()
        return code == 0
    def users(name='ordinary-services', packages=None, features=None):
        cmd=['cargo','build','--locked','--offline','--release']
        for p in packages or ['sunlight-init','sunlightd','sunlight-startup-fixture','sunlight-uac','sunlight-sessiond','sunlight-vfs-server']:
            cmd += ['-p',p]
        if features:
            cmd += ['--features',features]
        ok=check(name,cmd,{'RUSTFLAGS':SERVICE_FLAGS})
        if ok:
            staged={}
            for p in (BUILD/'x86_64-unknown-none/release').iterdir():
                if p.is_file() and p.read_bytes()[:4] == b'\x7fELF' and not p.suffix:
                    dest=ROOT/'target/x86_64-unknown-none/release'/p.name
                    shutil.copy2(p,dest)
                    staged[p.name]=digest(dest)
            summary.setdefault('staged_services',{})[name]=staged
            save()
        return ok
    def image(name, features, scratch, env=None):
        command=['cargo','build','--locked','--offline','-p','sunlight-kernel','--bin','sunlight-kernel']
        if features:
            command += ['--features',features]
        if not check(name+'-kernel',command,env):
            return None
        elf=Path(scratch)/(name+'.elf')
        iso=Path(scratch)/(name+'.iso')
        built=BUILD/'x86_64-unknown-none/debug/sunlight-kernel'
        if not check(name+'-strip',['objcopy','--strip-debug',str(built),str(elf)]):
            return None
        if not check(name+'-iso',['bash','tools/make_hybrid_iso.sh',str(elf),str(iso),str(ROOT/'target/limine'),scratch]):
            return None
        archive=BUILD/(out.name+'-'+name+'.iso.gz')
        with iso.open('rb') as source, gzip.open(archive,'wb',compresslevel=1) as dest:
            shutil.copyfileobj(source,dest)
        summary['images'][name]={'features':features,'kernel_sha256':digest(built),
            'elf_sha256':digest(elf),'iso_sha256':digest(iso),'archive':str(archive),'archive_sha256':digest(archive),
            'storage':'isolated /tmp artifacts; workspace disk nearly full; raw logs retained in evidence directory',
            'embedded_service_sha256':{p.name:digest(p) for p in (ROOT/'target/x86_64-unknown-none/release').iterdir()
                if p.is_file() and not p.suffix}}
        save()
        return iso
    def native(name, iso, cpu, markers, validator=None, no_entropy=False):
        if iso is None:
            summary['native'].append({'name':name,'cpus':cpu,'classification':'NOT_RUN','reason':'image build failed'})
            save()
            return
        serial=out/(name+'.serial.log'); serial.touch()
        cmd=['qemu-system-x86_64','-accel','tcg,thread=multi','-cpu','max,-rdrand,-rdseed' if no_entropy else 'max',
             '-smp',str(cpu),'-m','1024','-cdrom',str(iso),'-serial',f'file:{serial}','-display','none',
             '-nic','none','-no-reboot','-no-shutdown','-device','isa-debug-exit,iobase=0xf4,iosize=0x04']
        if not no_entropy: cmd += ['-device','virtio-rng-pci,disable-modern=on']
        start=time.monotonic()
        with (out/(name+'.qemu.log')).open('w') as log:
            proc=subprocess.Popen(cmd,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT)
            try:
                while proc.poll() is None and time.monotonic()-start < 180:
                    raw=serial.read_text(errors='replace')
                    if all(m in raw for m in markers) or 'PANIC' in raw or 'panicked at' in raw: break
                    time.sleep(.25)
            finally:
                if proc.poll() is None:
                    proc.terminate()
                    try: proc.wait(timeout=5)
                    except subprocess.TimeoutExpired: proc.kill()
                code=proc.wait()
        raw=serial.read_text(errors='replace')
        missing=[m for m in markers if m not in raw]
        detail=validator(raw,cpu) if validator else None
        good=not detail or detail.get('classification',detail.get('status')) in ('FORCED_AND_PASSED','PASS','pass')
        summary['native'].append({'name':name,'cpus':cpu,'command':cmd,'exit_code':code,
            'seconds':time.monotonic()-start,'classification':'PASS' if not missing and good and 'PANIC' not in raw and 'panicked at' not in raw else 'FAIL',
            'missing':missing,'validator':detail,'serial':str(serial.relative_to(ROOT)),
            'serial_sha256':digest(serial),'iso_sha256':digest(iso)})
        print(name,summary['native'][-1]['classification'],flush=True); save()
    try:
        if args.section in ('checks','all'):
            host=['cargo','test','--locked','--offline','--target','x86_64-unknown-linux-gnu']
            check('kernel-host',host+['-p','sunlight-kernel','--lib','--','--test-threads=1'])
            packages=['sunlight-ipc','sunlight-uac','sunlight-sessiond','sunlight-fs','sunlightd','sunlight-audiod','sunlight-elf','sunlight-fastlane-ring','sunlight-fastlane-doorbell']
            check('services-host',host+sum((['-p',p] for p in packages),[])+['--lib'])
            check('sunlightd-runtime-host',host+['-p','sunlightd','--bin','sunlightd','--features','dependency-test'])
            check('scheduler-host',['python3','tools/test_scheduler_host.py'])
            check('evidence-parsers',['python3','-m','unittest','discover','-s','tools','-p','test_fastlane_f11b0*.py'])
            check('strict-clippy',['cargo','clippy','--locked','--offline','--target','x86_64-unknown-linux-gnu','-p','sunlight-kernel','--lib','--','-D','warnings'])
            check('media-host',host+['-p','sunlight-media','--lib'])
            check('workspace-format',['cargo','fmt','--all','--','--check'])
            check('diff-check',['git','diff','--check'])
            check('miri-availability',['cargo','miri','--version'])
        if args.section != 'checks':
            if not users(): raise RuntimeError('ordinary services failed')
            with tempfile.TemporaryDirectory(prefix='f11b0e2-images-') as scratch:
                shutil.copy2(ROOT/'limine.conf',Path(scratch)/'limine.conf')
                if args.section in ('production','all'):
                    image('ordinary-production','',scratch)
                    users('production-probe-services',['sunlight-uac','sunlight-vfs-server'],'sunlight-uac/production-grant-test')
                    iso=image('production-probe','production_grant_test',scratch)
                    for cpu in (1,2,4,12):
                        native(f'production-{cpu}cpu',iso,cpu,['[GRANT-C] COMPLETE PASS'],assess_native)
                    users('restore-ordinary-services')
                if args.section in ('authority','all'):
                    iso=image('authority','cap_issuance_test',scratch,{'SUNLIGHT_CAP_EXEC_PROBE':'1'})
                    native('entropy-unavailable',iso,1,['[CAP] bootstrap BLOCKED: secure entropy unavailable; no user authority published'],no_entropy=True)
                    for cpu in (1,2,4,12):
                        native(f'authority-{cpu}cpu',iso,cpu,[f'[CAP-PR] lifecycle PASS auth=17 account=0 account_denied=35 post_reap_records=0 late_auth=denied late_account=denied cpus={cpu}','[SESSION-E2] boundary PASS'],assess_session)
                if args.section in ('core','all'):
                    for name,feature,marker,validator in [('pin-release','pin_release_test','[R2V] PASS cpus={cpu} ',validate_pin),('doorbell','native_doorbell_test','[F13NA] PASS ',validate_doorbell),('scheduler','scheduler_b_test','[SCHED-B] PASS cpus={cpu} ',None)]:
                        iso=image(name,feature,scratch)
                        for cpu in (1,2,4,12): native(f'{name}-{cpu}cpu',iso,cpu,[marker.format(cpu=cpu)],validator)
                    for name,feature,cpu,expected in [('vm-tlb','mm2b_smp_test',12,'mm2b.expected'),('vm-unmap','mm2d_munmap_test',4,'mm2d.expected'),('vm-protect','mm2e_mprotect_test',4,'mm2e.expected')]:
                        iso=image(name,feature,scratch)
                        native(name,iso,cpu,[m for m in (ROOT/'tools/tests'/expected).read_text().splitlines() if m and not m.startswith('#')])
                if args.section in ('lifecycle','all'):
                    users('dependency-services',['sunlightd','sunlight-startup-fixture'],'sunlightd/dependency-test,sunlight-startup-fixture/dependency-test')
                    iso=image('dependency','dependency_test',scratch)
                    if iso: check('d1-d2-d3',['python3','tools/sunlightd_d1_gate.py','--d2','--d3','--iso',str(iso),'--evidence',str(out/'d1-d2-d3'),'--cpus','1','2','4'])
                    users('recovery-services',['sunlight-init','sunlightd','sunlight-startup-fixture'],'sunlight-init/recovery-test,sunlightd/recovery-test,sunlight-startup-fixture/recovery-test')
                    iso=image('recovery','recovery_test',scratch,{'D36R_SCENARIO':'after-ready'})
                    if iso: check('recovery-blocked',['python3','tools/sunlightd_d1_gate.py','--d2','--d3','--d36r','--d36r-scenario','after-ready','--iso',str(iso),'--evidence',str(out/'recovery'),'--cpus','1','2','4'])
                    users('restore-ordinary-services')
                if args.section in ('network','all'):
                    iso=image('resident','fastlane_f1_audit',scratch)
                    native('resident-12cpu',iso,12,['[FASTLANE-F1] prerequisite lifetime gate: OK (channel remains BLOCKED)','[SunlightOS] sunlightd OK'])
                    if iso:
                        for cpu in (1,2,4,12): check(f'tcp-tls-{cpu}cpu',['python3','tools/network_tls_gate.py','--iso',str(iso),'--cpus',str(cpu),'--timeout','240','--evidence',str(out/f'tcp-tls-{cpu}cpu')],timeout=600)
    finally:
        summary['source_sha256_after']=sources()
        summary['source_hashes_unchanged']=summary['source_sha256']==summary['source_sha256_after']
        save(); print('Evidence:',out.relative_to(ROOT),flush=True)
    if not summary['source_hashes_unchanged'] or any(c['exit_code'] for c in summary['checks']) or any(r['classification']!='PASS' for r in summary['native']):
        raise SystemExit(1)

if __name__=='__main__': main()
