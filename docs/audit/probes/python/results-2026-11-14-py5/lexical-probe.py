from pathlib import Path
import subprocess,hashlib,json
R=Path.cwd(); E=Path(__file__).resolve().parent
shell=R/'sunshell/src/main.rs';files=R/'sunlight-files/src/main.rs'
s=shell.read_text();start=s.index('    fn normalize_path(');end=s.index('    fn external_uses_fs_paths',start)
f=files.read_text();a=f.index('#[derive(Clone, Copy)]\nstruct PathBuf');b=f.index('\n}',f.index('    fn as_str(&self)',a))+2
# Execute exact source slices, without importing host filesystem canonicalization.
code='extern crate alloc;\nconst PATH_LEN:usize=256;\n'+s[start:end]+f[a:b]+'''
fn main(){
 for (cwd,input) in [("/tmp","."),("/tmp",".."),("/","/tmp//./../tmp"),("/tmp","missing/../x"),("/tmp","relative"),("/tmp"," /tmp ")]{
  let sh=normalize_path(cwd,input).unwrap();let fm=PathBuf::from_str(input).unwrap();
  println!("cwd={cwd:?} input={input:?} shell={sh:?} files={:?}",fm.as_str());
 }
 assert_eq!(normalize_path("/tmp","/tmp//./../tmp").unwrap(),"/tmp");
 assert_eq!(PathBuf::from_str("/tmp//./../tmp").unwrap().as_str(),"/tmp");
 assert_eq!(normalize_path("/tmp","relative").unwrap(),"/tmp/relative");
 assert_eq!(PathBuf::from_str("relative").unwrap().as_str(),"/relative");
 assert_eq!(PathBuf::from_str("/tmp").unwrap().parent().unwrap().as_str(),"/");
 assert_eq!(PathBuf::from_str("/tmp").unwrap().join("x").unwrap().as_str(),"/tmp/x");
}
'''
p=E/'lexical-source-extract.rs';p.write_text(code)
cmd=['rustc','--edition=2021',str(p),'-o',str(R/'target/py5-lexical-probe')]
c=subprocess.run(cmd,capture_output=True,text=True);log=c.stdout+c.stderr
if c.returncode==0:
 x=subprocess.run([str(R/'target/py5-lexical-probe')],capture_output=True,text=True);log+=x.stdout+x.stderr
else:x=c
(E/'lexical-probe.log').write_text(log)
(E/'lexical-probe.json').write_text(json.dumps(dict(status='PASS' if x.returncode==0 else 'FAIL',command=cmd,source_hashes={str(p.relative_to(R)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [shell,files]},scope='Host execution of exact lexical source slices, not UI/native lookup or canonicalization',stdout=x.stdout),indent=2))
print(x.returncode)
