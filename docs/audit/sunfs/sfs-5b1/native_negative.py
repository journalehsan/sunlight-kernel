#!/usr/bin/env python3
"""Supplemental QEMU device/backend faults on the existing frozen SFS5B1 ISO."""
import datetime,gzip,hashlib,importlib.util,json,os,pathlib,re,subprocess,time
ROOT=pathlib.Path(__file__).resolve().parents[4]
OUT=pathlib.Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('runner',ROOT/'tools/sunfs5b1_validate.py');runner=importlib.util.module_from_spec(spec);spec.loader.exec_module(runner)

def write(p,obj): p.write_text(json.dumps(obj,indent=2)+'\n')
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    raw=OUT/'raw'/('native-negative-'+datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ'));raw.mkdir()
    frozen=runner.source();write(raw/'source-before.json',frozen)
    baseline=sorted((OUT/'raw').glob('native-*/cpu-1-writeback-initial.invocation.json'))[-1]
    original=json.loads(baseline.read_text());iso=pathlib.Path(original['argv'][original['argv'].index('-cdrom')+1]);assert sha(iso)==original['iso_sha256']
    artifacts=ROOT/'target/sfs5b1'/raw.name;artifacts.mkdir()
    cases=[('queue512','queue-size=512','', '[BLK]  virtio-blk init failed'),('logical4096','logical_block_size=4096,physical_block_size=4096','', '[BLK]  virtio-blk init failed'),('readonly','',',readonly=on','Rejected(ReadOnly)'),('flush-ioerr','', '', 'left: Failed')]
    results=[]
    for name,device_options,drive_options,expected in cases:
        image=artifacts/(name+'.img');initial=bytearray(8192*512);initial[:len(runner.MARKER)]=runner.MARKER
        with image.open('xb') as f: f.write(initial);f.flush();os.fsync(f.fileno())
        serial=raw/(name+'.serial.log');errors=raw/(name+'.qemu.log');backend=str(image)
        injection=None
        if name=='flush-ioerr':
            config=raw/'flush.conf';config.write_text('[inject-error]\nevent = "flush_to_disk"\nerrno = "5"\nonce = "on"\n')
            backend=f'blkdebug:{config}:{image}';injection={'event':'flush_to_disk','errno':5,'once':True,'config_sha256':sha(config)}
        argv=['qemu-system-x86_64','-machine','q35','-accel','tcg','-m','1024M','-smp','1','-cdrom',str(iso),'-boot','d','-display','none','-serial',f'file:{serial}','-monitor','none','-drive',f'id=negative,file={backend},if=none,format=raw,cache=writeback{drive_options}','-device','virtio-blk-pci,disable-modern=on,drive=negative,serial=SFS5B1-NEG'+(','+device_options if device_options else ''),'-device','virtio-rng-pci,disable-modern=on','-no-reboot','-no-shutdown','-nic','none']
        write(raw/(name+'.invocation.json'),{'argv':argv,'expected':expected,'injection':injection,'source_sha256':frozen,'runner_sha256':sha(ROOT/'tools/sunfs5b1_validate.py'),'harness_sha256':sha(pathlib.Path(__file__)),'iso_sha256':sha(iso),'elf_sha256':original['elf_sha256'],'cache':'writeback','configured_vcpus':1,'seed':'none: deterministic device option or blkdebug rule','input_image_sha256':sha(image)})
        content='';observed=False
        with errors.open('w') as error:
            p=subprocess.Popen(argv,cwd=ROOT,stdout=error,stderr=error)
            try:
                deadline=time.monotonic()+90
                while p.poll() is None and time.monotonic()<deadline:
                    content=serial.read_text(errors='replace') if serial.exists() else ''
                    # Wait for complete assertion text, not just the panic prefix.
                    if expected in content: observed=True;break
                    if '[SFS5A] READY-FOR-HOST-KILL' in content: break
                    time.sleep(.2)
            finally:
                if p.poll() is None:p.kill()
                code=p.wait(timeout=10)
        after=image.read_bytes();unchanged=after==initial
        row={'case':name,'status':'PASS' if observed else 'FAIL','expected':expected,'observed':expected if observed else 'expected marker absent','returncode':code,'serial':str(serial.relative_to(OUT)),'image_unchanged':unchanged,'image_sha256':sha(image),'actual_block_dma':name in ['readonly','flush-ioerr'],'completion_scope':'native expected assertion at fixture; full post-error receipt/owner dump unavailable' if name=='flush-ioerr' else 'native initialization guard' if name in ['queue512','logical4096'] else 'native typed pre-submission readonly refusal','retirement':'native state not directly printed; driver source + separate hosted assertions only','fencing_claim':False}
        if name!='flush-ioerr' and not unchanged: row['status']='FAIL'
        (raw/(name+'.img.gz')).write_bytes(gzip.compress(after,mtime=0));results.append(row)
        write(raw/'negative-native-result.json',{'status':'PASS' if all(x['status']=='PASS' for x in results) and len(results)==4 else 'PARTIAL','cases':results})
        print(name,row['status'],row['observed'],flush=True)
    final=runner.source();write(raw/'source-after.json',final);write(raw/'input-integrity.json',{'status':'PASS' if final==frozen else 'FAIL','changed':[n for n in set(final)|set(frozen) if final.get(n)!=frozen.get(n)]});assert final==frozen
    print(raw,flush=True)
if __name__=='__main__':main()
