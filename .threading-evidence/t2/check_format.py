import difflib,json,subprocess
from pathlib import Path
root=Path.cwd();old=root/'.threading-evidence/t2/initial/source'
files=['tools/tests/fd-description/src/lib.rs','sunlight-libc/src/mman.rs','kernel/src/process/mmap.rs','kernel/src/process/region.rs','sunlight-libc/src/thread.rs','kernel/src/process/thread_admission.rs','kernel/src/sched/thread_admission.rs','kernel/src/sched/t2_fixture.rs','kernel/src/process/mod.rs','kernel/src/process/env.rs','kernel/src/process/fd_table.rs','kernel/src/sched/mod.rs','kernel/src/arch/x86_64/syscall.rs','kernel/src/arch/x86_64/interrupts.rs','kernel/src/main.rs','kernel/src/mm2a_host.rs','kernel/src/process/spawn.rs','kernel/build.rs','sunlight-libc/src/std_runtime.rs','tools/native-std/overlay/thread/sunlightos.rs','sunlight-docserv/src/main.rs','tools/tests/scheduler_service.rs','tools/tests/thread_admission.rs']
for name in files:
 path=root/name
 if not (old/name).exists():
  subprocess.run(['rustfmt','--check','--edition','2021','--config','skip_children=true',str(path)],check=True)
 else:
  a=(old/name).read_text().splitlines();b=path.read_text().splitlines()
  lines=[{'file':str(path),'range':[j+1,max(j+1,k)]} for op,_,_,j,k in difflib.SequenceMatcher(a=a,b=b,autojunk=False).get_opcodes() if op!='equal']
  if lines:
   subprocess.run(['rustfmt','--check','--edition','2021','--config','skip_children=true','--unstable-features','--file-lines',json.dumps(lines),str(path)],check=True)
print('Changed spans and complete new files pass rustfmt checks.')
