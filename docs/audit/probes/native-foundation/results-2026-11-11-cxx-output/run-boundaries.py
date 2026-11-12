import subprocess,json
from pathlib import Path
r=Path('/home/ehsantor/Projects/sunlightos-kernel');o=r/'target/phase20'
expected='output 0 1 -9223372036854775808 9223372036854775807 18446744073709551615 18446744073709551615\nbounds -32768 32767 65535 -2147483648 2147483647 4294967295 -9223372036854775808 9223372036854775807 0\nABC\n'
for cpu in (2,4):
 dest=o/f'boundaries-{cpu}cpu'
 with (o/f'boundaries-{cpu}cpu-driver.log').open('w') as f:
  p=subprocess.run(['python3','tools/native-c/guest.py',str(o/'cxx-abi-final/cxx-output.elf'),'--cpus',str(cpu),'--expect-exit','0','--out',str(dest)],cwd=r,stdout=f,stderr=subprocess.STDOUT,timeout=900)
 assert p.returncode==0
 result=json.loads((dest/'results.json').read_text())['elf']
 assert result['pass_'] and result['stdout']==expected and result['stderr']=='stderr\nclog\n'
 (dest/'exact-output.json').write_text(json.dumps({'stdout':'PASS','stderr':'PASS','exit':0}))
