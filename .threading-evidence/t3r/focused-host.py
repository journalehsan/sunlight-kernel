import subprocess
runner=['python3','tools/threading_t3r_validate.py','command','--timeout','180']
for name,args in [('handoff-host-build',['rustc','--edition=2021','--test','kernel/src/sched/handoff.rs','-o','target/t3r-handoff-host']),('handoff-host',['target/t3r-handoff-host']),('requests-host-build',['rustc','--edition=2021','--test','kernel/src/sched/requests.rs','-o','target/t3r-requests-host']),('requests-host',['target/t3r-requests-host']),('admission-host',['python3','tools/threading_t2_host.py']),('scheduler-host',['python3','tools/test_scheduler_host.py'])]:
 code=subprocess.run(runner+[name,'--']+args).returncode
 print(name,code,flush=True)
 if code:raise SystemExit(code)
