from pathlib import Path
import subprocess
# Format only the changed function regions, preserving incoming file formatting.
def region(name,first,last,depth=0):
 p=Path(name);s=p.read_text();a=s.index(first);b=s.index(last,a) if last else len(s);chunk=s[a:b]
 wrapper=('mod fragment {\n'*depth)+chunk+('}\n'*depth)
 out=subprocess.check_output(['rustfmt','--edition','2021','--emit','stdout'],input=wrapper,text=True)
 lines=out.splitlines(True);out=''.join(lines[depth:len(lines)-depth] if depth else lines)
 p.write_text(s[:a]+out+'\n'+s[b:])
region('kernel/src/arch/x86_64/syscall.rs','/// Snapshot authority','fn sys_getcwd(')
region('sunlight-libc/src/lib.rs','/// Authoritative existing-object','/// Change the current process CWD')
region('sunlight-libc/src/c_file.rs','/// Native realpath',None)
region('sunshell/src/main.rs','    // Preserve components and whitespace','    fn normalize_path(',1)
region('sunshell/src/main.rs','        fn cmd_cd(', '        fn cmd_pwd(',2)
region('sunshell/src/native_script.rs','    fn directory(', '    fn source(',1)
region('sunlight-files/src/main.rs','    fn set(&mut self, text: &str)', '    fn join(',1)
region('sunlight-files/src/main.rs','    fn navigate_to(', '    fn load_directory(',1)
