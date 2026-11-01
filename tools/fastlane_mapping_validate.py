#!/usr/bin/env python3
"""M2 restricted native mapping evidence, including the M1 authority suite."""
import argparse
from datetime import datetime, timezone
import json
import mmap
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
from fastlane_f11b0e_validate import ROOT, digest, sources
from fastlane_f11b0c_validate import SERVICE_FLAGS
from voice_recorder_qmp import Qmp
BASE=ROOT/'target/fastlane-mapping-evidence'
BUILD=ROOT/'target/fastlane-f11b0e3-build'
ENV={'CARGO_TARGET_DIR':str(BUILD),'CARGO_INCREMENTAL':'0','CARGO_PROFILE_DEV_DEBUG':'0'}
SERVICES={'sunlight-init':'sunlight-init','sunlightd':'sunlightd','sunlight-tty-server':'sunlight-tty-server',
          'sunlight-uac':'uac_service','sunlight-sessiond':'sunlight-sessiond','sunlight-vfs-server':'sunlight-vfs-server','sunlightctl':'sunlightctl'}
CASES=('authorized-create','recipient-role-duplicate-stale','revoked-terminal','failed-exec-preserved',
       'replacement-denied','successful-exec-retired','peer-exit-incomplete','creator-exit-incomplete',
       'metadata-exhaustion','abandoned-expiry','complete')
def rows(raw,prefix):
    found=[]
    for line in raw.splitlines():
        if line.startswith(prefix):
            pairs=re.findall(r'(\w+)=(\S+)',line);r=dict(pairs)
            if len(r)!=len(pairs):r['_duplicate']='1'
            found.append(r)
    return found

from fastlane_channel_validate import assess as assess_m1
from fastlane_mapping_assess import assess_mapping

def assess(raw, cpus, identity=None, failure=None):
    first = raw.split('[FL-M2] case=start',1)[0]
    authority = assess_m1(first,cpus,identity,failure)
    mapping = assess_mapping(raw,cpus)
    errors = authority['errors'] + mapping['errors']
    return {'status':'FAIL' if errors else 'PASS','errors':errors,'authority':authority,'mapping':mapping}

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--section',choices=('build','native','production','checks'),required=True)
    p.add_argument('--cpus',nargs='+',type=int,choices=(2,4,12),default=[2]);p.add_argument('--image-manifest',type=Path)
    p.add_argument('--evidence-root',type=Path,default=BASE);p.add_argument('--timeout',type=int,default=180)
    args=p.parse_args();out=args.evidence_root.resolve()/(args.section+'-'+datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'));out.mkdir(parents=True)
    summary={'milestone':'M2','status':'RUNNING','revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
             'source_sha256':sources(),'checks':[],'native':[],'production_fastlane':'BLOCKED'}
    def save():(out/'validation-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    def check(name,cmd,env=None):
        with (out/(name+'.log')).open('w') as log:
            log.write(json.dumps({'command':cmd,'environment':{**ENV,**(env or {})}})+'\n');log.flush()
            r=subprocess.run(cmd,cwd=ROOT,env={**os.environ,**ENV,**(env or {})},stdout=log,stderr=subprocess.STDOUT)
        summary['checks'].append({'name':name,'command':cmd,'exit_code':r.returncode});save();print(name,r.returncode,flush=True)
        if r.returncode:raise RuntimeError(name+' failed')
    save();print(out,flush=True)
    try:
        if args.section in ('build','production'):
            test=args.section=='build'
            for package,binary in SERVICES.items():
                cmd=['cargo','build','--locked','--offline','--release','-p',package]
                if test and package=='sunlight-uac':cmd+=['--features','fastlane-mapping-test']
                check(package,cmd,{'RUSTFLAGS':SERVICE_FLAGS})
                shutil.copy2(BUILD/'x86_64-unknown-none/release'/binary,ROOT/'target/x86_64-unknown-none/release'/binary)
            if test:shutil.copy2(BUILD/'x86_64-unknown-none/release/session-race-probe',ROOT/'target/x86_64-unknown-none/release/session-race-probe')
            cmd=['cargo','build','--locked','--offline','-p','sunlight-kernel','--bin','sunlight-kernel']
            if test:cmd+=['--features','fastlane_mapping_test,key_inject']
            check('kernel',cmd,{'SUNLIGHT_INJECT_PHASE':'desktop_login'} if test else {})
            elf=out/'kernel.elf';iso=out/'fixture.iso'
            check('strip',['objcopy','--strip-debug',str(BUILD/'x86_64-unknown-none/debug/sunlight-kernel'),str(elf)])
            scratch=out/'iso-build';scratch.mkdir();shutil.copy2(ROOT/'limine.conf',scratch/'limine.conf')
            check('iso',['bash','tools/make_hybrid_iso.sh',str(elf),str(iso),str(ROOT/'target/limine'),str(scratch)])
            check('extract',['xorriso','-osirrox','on','-indev',str(iso),'-extract','/boot/sunlight-kernel.elf',str(out/'iso-kernel.elf')])
            assert digest(elf)==digest(out/'iso-kernel.elf')
            archive=out/'embedded';archive.mkdir();payloads={}
            with elf.open('rb') as stream,mmap.mmap(stream.fileno(),0,access=mmap.ACCESS_READ) as raw:
                for binary in list(SERVICES.values())+(['session-race-probe'] if test else []):
                    source=ROOT/'target/x86_64-unknown-none/release'/binary;data=source.read_bytes();offset=raw.find(data)
                    assert offset>=0,('service not embedded',binary)
                    shutil.copy2(source,archive/binary);payloads[binary]={'sha256':digest(source),'offset':offset,'bytes':len(data)}
                assert (raw.find(b'[FL-AUTH]') >= 0)==test, 'authority feature isolation mismatch'
                assert (raw.find(b'[FL-RACE]') >= 0)==test, 'race feature isolation mismatch'
                assert (raw.find(b'[FL-USER]') >= 0)==test, 'userspace feature isolation mismatch'
                for marker in (b'[FL-MAP]',b'[FL-TLB]',b'[FL-FRAME]',b'[FL-M2]',b'[FL-PF]'):
                    assert (raw.find(marker)>=0)==test, ('mapping isolation mismatch',marker)
            summary['image']={'elf':str(elf),'iso':str(iso),'elf_sha256':digest(elf),'iso_sha256':digest(iso),
                              'features':['fastlane_mapping_test','key_inject'] if test else [],'services':payloads}
        elif args.section=='checks':
            host=['cargo','test','--locked','--offline','--target','x86_64-unknown-linux-gnu']
            check('kernel-host',host+['-p','sunlight-kernel','--lib','--','--test-threads=1'])
            check('ipc-host',host+['-p','sunlight-ipc','--lib'])
            check('uac-host',host+['-p','sunlight-uac','--lib'])
            check('scheduler-host',['python3','tools/test_scheduler_host.py'])
            check('parser-controls',['python3','-m','unittest','discover','-s','tools','-p','test_fastlane_mapping.py'])
            check('diff',['git','diff','--check'])
        else:
            if not args.image_manifest:raise RuntimeError('--image-manifest required')
            manifest=json.loads(args.image_manifest.read_text());image=manifest['image'];summary['image']=image
            assert manifest['status']=='PASS' and manifest['source_sha256']==sources(),'source mismatch'
            assert image['features']==['fastlane_mapping_test','key_inject']
            identity={'source':True,'elf':digest(Path(image['elf']))==image['elf_sha256'],'iso':digest(Path(image['iso']))==image['iso_sha256'],
                      'services':all(digest(Path(image['elf']).parent/'embedded'/name)==row['sha256'] for name,row in image['services'].items())}
            assert all(identity.values()),'image mismatch'
            for cpus in args.cpus:
                serial=out/f'{cpus}cpu.serial.log';serial.write_text('');sock=Path('/tmp')/f'fl-channel-{out.name}-{cpus}.sock'
                command=['qemu-system-x86_64','-accel','tcg,thread=multi','-cpu','max','-smp',str(cpus),'-m','2048','-cdrom',image['iso'],
                         '-serial','file:'+str(serial),'-display','none','-nic','none','-no-reboot','-no-shutdown',
                         '-device','virtio-rng-pci,disable-modern=on','-qmp','unix:'+str(sock)+',server=on,wait=off']
                start=time.monotonic();failure=None;qmp=None
                def raw():return serial.read_text(errors='replace')
                def await_marker(marker):
                    while time.monotonic()-start<args.timeout and proc.poll() is None:
                        data=raw()
                        if marker in data:return
                        if any(m in data for m in ('PANIC','panicked at','checkpoint=FAIL')):raise RuntimeError('guest assertion')
                        time.sleep(.1)
                    if proc.poll() is not None:raise RuntimeError('QEMU exited before checkpoint: '+marker)
                    raise TimeoutError(marker)
                def key(*codes):
                    for down,ordered in ((True,codes),(False,tuple(reversed(codes)))):
                        for code in ordered:qmp.call('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':code}}}]})
                        time.sleep(.08)
                def type_text(value):
                    for char in value:
                        if char.isupper():key('shift',char.lower())
                        else:key({' ':'spc','/':'slash','-':'minus','\n':'ret'}.get(char,char))
                with (out/f'{cpus}cpu.qemu.log').open('w') as log:
                    proc=subprocess.Popen(command,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT)
                    try:
                        await_marker('[SESSION] switched to F2 GraphicalDesktop');qmp=Qmp(str(sock));key('ctrl','f1')
                        await_marker('[SESSION] switched to F1 TTY/Login');key('spc');key('ret')
                        await_marker('[TTY]  Built-in shell ready');await_marker("sunshell registered as 'sshl'")
                        type_text('touch /tmp/session-race-probe\n');time.sleep(.5)
                        type_text('/tmp/session-race-probe FL\n');await_marker('exit: /tmp/session-race-probe FL -> 0')
                        complete=rows(raw(),'[FL-USER] ');complete=[r for r in complete if r.get('case')=='complete']
                        if len(complete)!=1:raise RuntimeError('missing terminal user receipt')
                        await_marker('[SCHED] process_reaped pid='+complete[0]['pid']+' ')
                        type_text('/tmp/session-race-probe FL-M2\n')
                        await_marker('exit: /tmp/session-race-probe FL-M2 -> 0')
                        terminal=[r for r in rows(raw(),'[FL-M2] ') if r.get('case')=='complete']
                        if len(terminal)!=1:raise RuntimeError('missing M2 terminal receipt')
                        await_marker('[SCHED] process_reaped pid='+terminal[0]['pid']+' ')

                    except Exception as error:failure={'type':type(error).__name__,'message':str(error)}
                    finally:
                        if qmp:qmp.stream.close();qmp.socket.close()
                        if proc.poll() is None:
                            proc.terminate()
                            try:proc.wait(timeout=5)
                            except subprocess.TimeoutExpired:proc.kill()
                        proc.wait()
                assessment=assess(raw(),cpus,identity,failure)
                row={'cpus':cpus,'command':command,'failure':failure,'assessment':assessment,'serial':str(serial),'serial_sha256':digest(serial)}
                if assessment['status']=='PASS':
                    ack={'cpus':cpus,'serial_sha256':digest(serial),'image':image,'identity':identity,'security_assertions':'PASS','cleanup':'PASS'}
                    (out/f'{cpus}cpu.host-ack.json').write_text(json.dumps(ack,indent=2)+'\n')
                if 'mapping' in assessment:
                    (out/f'{cpus}cpu.physical-objects.json').write_text(json.dumps(assessment['mapping'],indent=2)+'\n')
                summary['native'].append(row);save();print('native',cpus,assessment['status'],assessment['errors'],flush=True)
                if assessment['status']!='PASS':raise RuntimeError('native failed; retained original attempt')
        summary['status']='PASS'
    except Exception as error:
        summary['status']='FAIL';summary['failure']={'type':type(error).__name__,'message':str(error)}
    finally:
        summary['source_sha256_after']=sources()
        if summary['source_sha256_after']!=summary['source_sha256']:summary['status']='FAIL';summary['source_changed']=True
        save()
        archive=out/'source';archive.mkdir(exist_ok=True)
        for name in summary['source_sha256']:
            src=ROOT/name;dest=archive/name;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(src,dest)
        (out/'artifact-index.json').write_text(json.dumps({str(p.relative_to(out)):digest(p) for p in out.rglob('*') if p.is_file() and p.name!='artifact-index.json'},indent=2)+'\n')
    print('status',summary['status'],out,flush=True)
    return 0 if summary['status']=='PASS' else 1
if __name__=='__main__':raise SystemExit(main())
