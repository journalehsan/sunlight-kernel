#!/usr/bin/env python3
"""M1 authority-only evidence. Real Account login and independent userspace actors."""
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
BASE=ROOT/'target/fastlane-channel-evidence'
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

def assess(raw,cpus,identity=None,failure=None):
    errors=[]
    def require(ok,why):
        if not ok:errors.append(why)
    users=rows(raw,'[FL-USER] ');auth=rows(raw,'[FL-AUTH] ');races=rows(raw,'[FL-RACE] ');outcomes=rows(raw,'[FL-OUTCOME] ');bindings=rows(raw,'[FL-BIND] ')
    require(not failure,'native driver failure: '+str(failure))
    require(identity and all(identity.get(k) for k in ('source','elf','iso','services')),'missing source/image/service identity')
    require(not any(s in raw for s in ('PANIC','panicked at','[FAULT]','checkpoint=FAIL')),'guest assertion/fault')
    require(not any('_duplicate' in r for r in users+auth+races+outcomes+bindings),'duplicate trace field')
    require(f'[SMP] Phase-1 bring-up complete: {cpus-1} AP(s) online ({cpus} total logical CPUs)' in raw,'CPU topology absent')
    for case in CASES:
        matches=[r for r in users if r.get('case')==case]
        require(len(matches)==1 and matches[0].get('result')=='0','missing/failed/duplicate case '+case)
    # Real production login/session admission, before new channel authorization.
    sessions=rows(raw,'[SESSION-E3] ')
    for event in ('account-activated','login-proof-committed','registered','transition-committed'):
        require(any(r.get('checkpoint')==event for r in sessions),'authentication checkpoint missing: '+event)
    require(any(r.get('op')=='0' and r.get('result')=='0' for r in auth),'producer proposal authorization missing')
    require(any(r.get('op')=='1' and r.get('result')=='0' for r in auth),'consumer consent authorization missing')
    require(any(r.get('op')=='2' and r.get('result')=='0' for r in auth),'creator syscall missing')
    require(any(r.get('op')=='3' and r.get('result')=='3' for r in auth),'duplicate attach rejection missing')
    require(any(r.get('op')=='3' and r.get('result')=='1' for r in auth),'foreign/role rejection missing')
    require(any(r.get('op')=='3' and r.get('result')=='2' for r in auth),'stale handle rejection missing')
    require(any(r.get('op')=='3' and r.get('result')=='4' for r in auth),'terminal rejection missing')
    for r in auth+races+outcomes+bindings:
        require(r.get('cpu','').isdigit() and 0<=int(r['cpu'])<cpus,'invalid participating CPU')
    require(bool(bindings),'kernel participant bindings missing')
    def same_identity(a,prefix_a,b,prefix_b):
        return all(a.get(prefix_a+suffix)==b.get(prefix_b+suffix) for suffix in ('','_inc','_vm'))
    proposed={r.get('generation'):r for r in bindings if r.get('op')=='0'}
    consented={r.get('generation'):r for r in bindings if r.get('op')=='1'}
    created={r.get('generation'):r for r in bindings if r.get('op')=='2'}
    for op,table in (('0',proposed),('1',consented),('2',created)):
        require(len(table)==sum(r.get('op')==op for r in bindings),'duplicate binding transition '+op)
    registrations=[r for r in sessions if r.get('checkpoint')=='registered']
    issuers=[r for r in sessions if r.get('checkpoint')=='account-activated']
    for binding in bindings:
        require(all(binding.get(k,'').isdigit() and int(binding[k])>0 for k in
                    ('generation','issuer','issuer_inc','issuer_vm','session','epoch','producer','producer_inc','producer_vm')),'invalid exact binding identity')
        require(any(r.get('session')==binding.get('session') and r.get('epoch')==binding.get('epoch') for r in registrations),'binding lacks real registered session')
        require(any(r.get('pid')==binding.get('issuer') and r.get('incarnation')==binding.get('issuer_inc') and r.get('vm')==binding.get('issuer_vm') for r in issuers),'binding issuer differs from authenticated Account image')
        require(binding.get('state') in ({'0':('0',),'1':('1',),'2':('2',),'3':('2','3')}.get(binding.get('op'),())),'incorrect binding state transition')
        if binding.get('op') in ('0','1'):
            require(same_identity(binding,'actor',binding,'issuer'),'authorization did not execute as exact issuer')
        if binding.get('op')!='0':
            original=proposed.get(binding.get('generation'),{})
            require(same_identity(binding,'producer',original,'producer') and same_identity(binding,'issuer',original,'issuer')
                    and all(binding.get(k)==original.get(k) for k in ('session','epoch')),'proposal issuer/producer/session changed')
            require(all(binding.get(k,'').isdigit() and int(binding[k])>0 for k in ('consumer','consumer_inc','consumer_vm'))
                    and binding.get('producer')!=binding.get('consumer') and binding.get('producer_vm')!=binding.get('consumer_vm'),'consumer is not independent exact VM')
        if binding.get('op') in ('2','3'):
            consent=consented.get(binding.get('generation'),{})
            require(same_identity(binding,'consumer',consent,'consumer'),'consumer reassigned or missing independent consent')
        if binding.get('op')=='2':
            require(same_identity(binding,'actor',binding,'producer') and binding.get('state')=='2','creation not exact producer or wrong state')
        if binding.get('op')=='3':
            role=binding.get('role');prefix={'0':'producer','1':'consumer'}.get(role,'invalid')
            require(same_identity(binding,'actor',binding,prefix) and binding.get('generation') in created,'successful attachment by wrong/stale role')
    attaches=[r for r in bindings if r.get('op')=='3']
    keys=[(r.get('generation'),r.get('role')) for r in attaches]
    require(len(keys)==len(set(keys)),'role attached successfully more than once')
    require(any({role for generation,role in keys if generation==g}=={'0','1'} for g in created),'both valid role attachments absent')
    for mode in range(1,5):
        selected=[r for r in races if r.get('mode')==str(mode)]
        arms=[r for r in selected if r.get('event')=='armed']
        require(len(arms)==1,'race arm missing/duplicate '+str(mode))
        if len(arms)!=1:continue
        generation=arms[0].get('generation')
        snaps=[r for r in outcomes if r.get('generation')==generation]
        require(len(selected)==7 and len(snaps)==2,'race transition count '+str(mode))
        def one(event,actor=None):
            found=[r for r in selected if r.get('event')==event and (actor is None or r.get('actor')==str(actor))]
            require(len(found)==1,'missing/duplicate race '+event+'/'+str(actor))
            return found[0] if len(found)==1 else {}
        e1=one('entered',1);e2=one('entered' if mode<=2 else 'exit-syscall',2)
        held=one('lock-held');contended=one('lock-contended');ends=[one('returned',a) for a in (1,2)]
        # Six events plus arm: arm, two entries, held, contention, two returns.
        require(e1.get('cpu')!=e2.get('cpu'),'syscalls did not overlap on separate CPUs')
        winner=1 if mode%2 else 2
        require(held.get('actor')==str(winner) and contended.get('actor')==str(3-winner),'wrong/missing real lock contention')
        if all(r.get('seq','').isdigit() for r in (e1,e2,held,contended,*ends)):
            require(max(int(e1['seq']),int(e2['seq']))<int(held['seq'])<int(contended['seq'])<min(int(r['seq']) for r in ends),'race invocation/lock ordering')
            require(int(ends[winner-1]['seq'])<int(ends[2-winner]['seq']),'wrong winning transition')
        require(ends[0].get('result')==('0' if winner==1 else '4') and ends[1].get('result')=='0','race results mismatch')
        require(all(r.get('generation')==generation for r in selected),'race mixed generations')
        last=[r for r in snaps if r.get('actor')==str(3-winner)]
        require(len(last)==1 and last[0].get('state')=='4' and last[0].get('terminal')==('1' if mode<=2 else '2'),'one terminal outcome missing')
        require(any(r.get('case')=='contended-race' and r.get('result')==str(mode) for r in users),'userspace race completion missing')
    complete=[r for r in users if r.get('case')=='complete']
    if complete:
        require(re.search(r'\[SCHED\] process_reaped pid='+re.escape(complete[0]['pid'])+r'\b',raw),'supervisor not reaped')
    require('[SYSCALL] exec: path not found: /missing-channel-exec' in raw,'failed exec syscall missing')
    require('exit: /tmp/session-race-probe FL -> 0' in raw,'shell did not collect successful fixture exit')
    return {'status':'FAIL' if errors else 'PASS','errors':errors,'cpus':cpus,'scope':'authority only; no mappings/data/Doorbell',
            'native_generation_exhaustion':'NOT_FORGED; host boundary only','race_modes':4,
            'participating_cpus':sorted({int(r['cpu']) for r in races if r.get('cpu','').isdigit()})}

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--section',choices=('build','native','production','checks'),required=True)
    p.add_argument('--cpus',nargs='+',type=int,choices=(2,4,12),default=[2]);p.add_argument('--image-manifest',type=Path)
    p.add_argument('--evidence-root',type=Path,default=BASE);p.add_argument('--timeout',type=int,default=180)
    args=p.parse_args();out=args.evidence_root.resolve()/(args.section+'-'+datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'));out.mkdir(parents=True)
    summary={'milestone':'M1','status':'RUNNING','revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
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
                if test and package=='sunlight-uac':cmd+=['--features','fastlane-channel-test']
                check(package,cmd,{'RUSTFLAGS':SERVICE_FLAGS})
                shutil.copy2(BUILD/'x86_64-unknown-none/release'/binary,ROOT/'target/x86_64-unknown-none/release'/binary)
            if test:shutil.copy2(BUILD/'x86_64-unknown-none/release/session-race-probe',ROOT/'target/x86_64-unknown-none/release/session-race-probe')
            cmd=['cargo','build','--locked','--offline','-p','sunlight-kernel','--bin','sunlight-kernel']
            if test:cmd+=['--features','fastlane_channel_test,key_inject']
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
            summary['image']={'elf':str(elf),'iso':str(iso),'elf_sha256':digest(elf),'iso_sha256':digest(iso),
                              'features':['fastlane_channel_test','key_inject'] if test else [],'services':payloads}
        elif args.section=='checks':
            host=['cargo','test','--locked','--offline','--target','x86_64-unknown-linux-gnu']
            check('kernel-host',host+['-p','sunlight-kernel','--lib','--','--test-threads=1'])
            check('ipc-host',host+['-p','sunlight-ipc','--lib'])
            check('uac-host',host+['-p','sunlight-uac','--lib'])
            check('scheduler-host',['python3','tools/test_scheduler_host.py'])
            check('parser-controls',['python3','-m','unittest','discover','-s','tools','-p','test_fastlane_channel.py'])
            check('diff',['git','diff','--check'])
        else:
            if not args.image_manifest:raise RuntimeError('--image-manifest required')
            manifest=json.loads(args.image_manifest.read_text());image=manifest['image'];summary['image']=image
            assert manifest['status']=='PASS' and manifest['source_sha256']==sources(),'source mismatch'
            assert image['features']==['fastlane_channel_test','key_inject']
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
