#!/usr/bin/env python3
"""Host controls compile actual production completion/wake/expiry helpers.
Hardware placement is stubbed; native fixtures prove dispatch/reap separately.
Generated Rust is retained in the selected evidence directory.
"""
import argparse
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]

def function(source, name):
    start = source.index('fn ' + name + '(')
    end = source.index('{', start) + 1
    depth = 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]

PREFIX = r'''
#![allow(dead_code)]
use std::sync::{Arc, Mutex, Barrier, atomic::{AtomicU64, Ordering}};
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum ProcessState { Running, Ready, BlockedOnIo, BlockedOnIpc, BlockedOnTimer, Suspended, Finished, Reaped }
mod process { pub mod mm2b_state { #[derive(Clone,Copy,Debug,PartialEq,Eq)] pub struct AddressSpaceIdentity(pub u64); } }
use process::mm2b_state::AddressSpaceIdentity;
struct AddressSpace(AddressSpaceIdentity);
impl AddressSpace { fn identity(&self)->AddressSpaceIdentity { self.0 } }
#[derive(Default)] struct Linux { futex_wait: Option<u64>, poll_wake_tick: Option<u64> }
struct Process { pid:usize, incarnation:u64, address_space:AddressSpace, state:ProcessState,
 native_futex_wait:Option<u64>, native_futex_deadline:Option<u64>, native_futex_result:Option<(u64,u64)>, futex_return_pending:Option<(u64,bool)>,
 linux:Option<Linux>, priority_wake_pending:bool, block_start_tick:u64,
 tty_write_wait:Option<u64>, tty_read_wait:Option<u64>, owner:Option<usize>, cpu:usize }
impl Process {
 fn linux_state(&self)->Option<&Linux>{self.linux.as_ref()}
 fn linux_state_mut(&mut self)->Option<&mut Linux>{self.linux.as_mut()}
'''
MIDDLE = r'''
}
mod timekeeping { use super::*; pub static NOW:AtomicU64=AtomicU64::new(0); pub fn monotonic_ns()->u64{NOW.load(Ordering::SeqCst)} }
static REQUESTS:AtomicU64=AtomicU64::new(0);
fn request_reschedule_on(cpu:usize){REQUESTS.fetch_or(1<<cpu,Ordering::SeqCst);}
struct Scheduler{processes:Vec<Process>, queues:Vec<Vec<usize>>, global_tick:u64}
impl Scheduler {
 fn live_owner_core(&self,i:usize)->Option<usize>{self.processes[i].owner}
 fn remove_from_ready_queues(&mut self,i:usize){for q in &mut self.queues{q.retain(|&j|j!=i);}}
 fn enqueue_ready(&mut self,i:usize){let cpu=self.processes[i].cpu; self.queues[cpu].push(i);}
 fn process_index_by_pid(&self,pid:usize)->Option<usize>{self.processes.iter().position(|p|p.pid==pid)}
'''
TESTS = r'''
}
fn fixture(n:usize)->Scheduler {
 Scheduler{processes:(0..n).map(|i|Process{pid:100+i,incarnation:1000+i as u64,
 address_space:AddressSpace(AddressSpaceIdentity(7)),state:ProcessState::BlockedOnIo,
 native_futex_wait:Some(4096),native_futex_deadline:Some(10),native_futex_result:None,futex_return_pending:None,
 linux:None,priority_wake_pending:false,block_start_tick:0,tty_write_wait:None,tty_read_wait:None,
 owner:None,cpu:0}).collect(),queues:vec![vec![],vec![]],global_tick:0}
}
fn verify(s:&Scheduler,i:usize,result:u64){let p=&s.processes[i];assert_eq!(p.pid,100+i);assert_eq!(p.incarnation,1000+i as u64);
 assert_eq!(p.state,ProcessState::Ready);assert_eq!(p.native_futex_wait,None);assert_eq!(p.native_futex_deadline,None);
 assert_eq!(p.native_futex_result,Some((4096,result)));assert_eq!(s.queues.iter().flatten().filter(|&&j|j==i).count(),usize::from(p.owner.is_none()));}
fn wake(s:&mut Scheduler,n:usize)->usize{s.wake_native_futex(AddressSpaceIdentity(7),4096,n)}
fn expire(s:&mut Scheduler){timekeeping::NOW.store(10,Ordering::SeqCst);s.expire_native_futex_timeouts();}
fn row(case:&str){println!("{{\"control\":\"{}\",\"status\":\"PASS\",\"scope\":\"production helpers; placement stub\"}}",case);}
fn main(){
 let mut s=fixture(1);assert_eq!(wake(&mut s,1),1);expire(&mut s);verify(&s,0,0);row("C1");
 let mut s=fixture(1);expire(&mut s);assert_eq!(wake(&mut s,1),0);verify(&s,0,2);row("C2");
 for cancellation in [false,true] {
  // Both deterministic winner orders, then barrier-started real host contention.
  for order in 0..66 {
   let shared=Arc::new(Mutex::new(fixture(1))); let barrier=Arc::new(Barrier::new(3));
   if order<2 {let mut s=shared.lock().unwrap();if order==0 {if cancellation{s.wake_for_signal(100)}else{wake(&mut s,1);}}else{expire(&mut s);}}
   let a=shared.clone();let b=barrier.clone();let first=std::thread::spawn(move||{b.wait();let mut s=a.lock().unwrap();if cancellation{s.wake_for_signal(100);0}else{wake(&mut s,1)}});
   let a=shared.clone();let b=barrier.clone();let second=std::thread::spawn(move||{b.wait();expire(&mut a.lock().unwrap());});
   barrier.wait();let selected=first.join().unwrap();second.join().unwrap();assert_eq!(Arc::strong_count(&shared),1);
   let s=shared.lock().unwrap();let result=s.processes[0].native_futex_result.unwrap().1;
   assert!(result==0||result==2);if order<2{assert_eq!(result,if order==0{0}else{2});}
   if !cancellation&&order>=2{assert_eq!(selected,usize::from(result==0));}verify(&s,0,result);
  }row(if cancellation{"C6"}else{"C3"});
 }
 let mut s=fixture(1);s.wake_for_signal(100);assert_eq!(wake(&mut s,1),0);verify(&s,0,0);row("C4");
 let mut s=fixture(1);assert_eq!(wake(&mut s,1),1);s.wake_for_signal(100);verify(&s,0,0);row("C5");
 let mut s=fixture(1);s.processes[0].clear_futex_wait();s.processes[0].state=ProcessState::Finished;
 assert_eq!(wake(&mut s,1),0);expire(&mut s);assert!(s.queues.iter().all(Vec::is_empty));assert_eq!(s.processes[0].native_futex_result,None);
 s.processes[0].incarnation+=1;s.processes[0].pid+=1000;assert_eq!(wake(&mut s,1),0);row("C7");
 let mut s=fixture(1);s.processes[0].cpu=1;assert_eq!(wake(&mut s,1),1);assert_eq!(s.queues[1],vec![0]);verify(&s,0,0);row("C8");
 for linux in [false,true]{
  let mut s=fixture(34);for i in 0..34{if linux{s.processes[i].native_futex_wait=None;s.processes[i].native_futex_deadline=None;s.processes[i].linux=Some(Linux{futex_wait:Some(4096),poll_wake_tick:None});}}
  s.processes[32].address_space=AddressSpace(AddressSpaceIdentity(8));
  if linux{s.processes[33].linux.as_mut().unwrap().futex_wait=Some(8192)}else{s.processes[33].native_futex_wait=Some(8192)};
  let shared=Arc::new(Mutex::new(s));let gate=Arc::new(Barrier::new(5));
  let handles:Vec<_>=(0..4).map(|_|{let a=shared.clone();let b=gate.clone();std::thread::spawn(move||{b.wait();let mut counts=vec![];for limit in [0,1,4,1,4,1,32,1]{let mut s=a.lock().unwrap();let n=if linux{s.wake_linux_futex(AddressSpaceIdentity(7),4096,limit)}else{wake(&mut s,limit)};assert!(n<=limit);counts.push(n);}counts})}).collect();
  gate.wait();let selected:usize=handles.into_iter().flat_map(|h|h.join().unwrap()).sum();assert_eq!(selected,32);assert_eq!(Arc::strong_count(&shared),1);
  let s=shared.lock().unwrap();for i in 0..32{if !linux{verify(&s,i,0)}else{assert_eq!(s.processes[i].linux_state().unwrap().futex_wait,None);assert_eq!(s.queues.iter().flatten().filter(|&&j|j==i).count(),1);}}
  for i in 32..34{assert_eq!(s.processes[i].state,ProcessState::BlockedOnIo);}assert_eq!(s.queues.iter().flatten().count(),32);
 }row("C9/D");
 let mut s=fixture(1);for cycle in 0..256{s.remove_from_ready_queues(0);let p=&mut s.processes[0];p.state=ProcessState::BlockedOnIo;p.native_futex_wait=Some(4096);p.native_futex_deadline=Some(10);p.native_futex_result=None;expire(&mut s);assert_eq!(wake(&mut s,1),0);verify(&s,0,2);assert_eq!(s.processes[0].native_futex_result.take(),Some((4096,2)));assert_eq!(s.processes[0].native_futex_result.take(),None);assert!(cycle<256);}row("C10");
 // C11 native compare/enqueue boundary is inherited from the unchanged T4A syscall.
 let mut s=fixture(1);s.wake_for_signal(100);assert_eq!(wake(&mut s,1),0);expire(&mut s);verify(&s,0,0);row("C12");
 println!("{{\"control\":\"EINTR/pthread_cancel\",\"status\":\"UNSUPPORTED\"}}");
}
'''

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('evidence',type=Path);a=p.parse_args()
    a.evidence.mkdir(parents=True,exist_ok=False)
    process=(ROOT/'kernel/src/process/mod.rs').read_text();sched=(ROOT/'kernel/src/sched/mod.rs').read_text()
    code=PREFIX+'\n'.join(function(process,n) for n in ('clear_futex_wait','complete_futex_wait'))+MIDDLE
    code+='\n'.join(function(sched,n) for n in ('wake_native_futex','wake_native_futex_index','wake_linux_futex','wake_linux_futex_index','expire_native_futex_timeouts','record_wakeup','wake_for_signal'))+TESTS
    path=a.evidence/'production-helpers.rs';path.write_text(code)
    binary=a.evidence/'production-helpers'
    subprocess.run(['rustc','--edition=2021',str(path),'-o',str(binary)],check=True,cwd=ROOT)
    subprocess.run([str(binary.resolve())],check=True,cwd=ROOT,timeout=60)

    # Compile the unchanged production syscall admission and RESULT implementation.
    # VM lookup and post-SYSRET trap are host adapters, explicitly outside this proof.
    admission = r'''
#[derive(Clone,Copy)]struct VirtAddr;impl VirtAddr{fn new(_:u64)->Self{Self}}
struct Response{offset:u64}struct Hhdm;static HHDM_REQ:Hhdm=Hhdm;
impl Hhdm{fn response(&self)->Option<Response>{Some(Response{offset:0})}}
const ERR_EINVAL:u64=u64::MAX-5;
static WORD:std::sync::atomic::AtomicU32=std::sync::atomic::AtomicU32::new(0);
struct SyscallFrame{rdi:u64,rsi:u64,rdx:u64,r10:u64,rcx:u64}
mod memory{pub mod user{pub fn read_process_futex_u32(_: &super::super::Process,_:super::super::VirtAddr,address:u64)->Result<u32,()>{if address!=4096{return Err(());}Ok(super::super::WORD.load(std::sync::atomic::Ordering::Acquire))}}}
mod sched{use super::*;pub struct Global(Mutex<Scheduler>);impl Global{pub fn lock(&self)->std::sync::MutexGuard<'_,Scheduler>{self.0.lock().unwrap()}}
pub static SCHEDULER:Global=Global(Mutex::new(Scheduler{processes:vec![],queues:vec![],global_tick:0}));pub fn request_reschedule_after_sysret(){}}
impl Scheduler{fn current_process(&self)->&Process{&self.processes[0]}fn current_process_mut(&mut self)->&mut Process{&mut self.processes[0]}fn current_process_index(&self)->Option<usize>{Some(0)}fn set_state(&mut self,i:usize,state:ProcessState){self.processes[i].state=state;}}
fn linux_errno(n:u64)->u64{(-(n as i64)) as u64}
impl Process{fn is_linux_compat(&self)->bool{self.linux.is_some()}fn is_exiting(&self)->bool{matches!(self.state,ProcessState::Finished|ProcessState::Reaped)}}
fn main(){
 *sched::SCHEDULER.lock()=fixture(1);{let mut s=sched::SCHEDULER.lock();s.processes[0].clear_futex_wait();s.processes[0].state=ProcessState::Running;}
 let mut frame=SyscallFrame{rdi:4096,rsi:0,rdx:0,r10:u64::MAX,rcx:100};
 WORD.store(1,Ordering::Release);assert_eq!(sys_native_futex(&mut frame),1);
 {let s=sched::SCHEDULER.lock();assert_eq!(s.processes[0].state,ProcessState::Running);assert_eq!(s.processes[0].native_futex_wait,None);assert_eq!(s.processes[0].native_futex_result,None);}
 WORD.store(0,Ordering::Release);assert_eq!(sys_native_futex(&mut frame),155);assert_eq!(frame.rcx,98);
 for _ in 0..8{frame.rcx=100;assert_eq!(sys_native_futex(&mut frame),155);assert_eq!(frame.rcx,98);let s=sched::SCHEDULER.lock();assert_eq!(s.processes[0].native_futex_wait,Some(4096));assert_eq!(s.processes[0].native_futex_result,None);}
 {let mut s=sched::SCHEDULER.lock();assert_eq!(s.processes[0].native_futex_wait,Some(4096));assert_eq!(wake(&mut s,1),1);verify(&s,0,0);}
 WORD.store(1,Ordering::Release);assert_eq!(sys_native_futex(&mut frame),0);assert_eq!(sched::SCHEDULER.lock().processes[0].futex_return_pending,None);
 frame.rsi=2;frame.rdi=8192;assert_eq!(sys_native_futex(&mut frame),ERR_EINVAL);frame.rdi=4096;assert_eq!(sys_native_futex(&mut frame),0);assert_eq!(sys_native_futex(&mut frame),ERR_EINVAL);
 // Deterministic delayed-IPI control: expiry can win while WAIT is replaying.
 {let mut s=sched::SCHEDULER.lock();*s=fixture(1);s.processes[0].clear_futex_wait();s.processes[0].state=ProcessState::Running;}
 frame.rsi=0;frame.r10=20;timekeeping::NOW.store(0,Ordering::SeqCst);WORD.store(0,Ordering::Release);assert_eq!(sys_native_futex(&mut frame),155);
 {let mut s=sched::SCHEDULER.lock();timekeeping::NOW.store(20,Ordering::SeqCst);s.expire_native_futex_timeouts();assert_eq!(wake(&mut s,1),0);verify(&s,0,2);}
 WORD.store(1,Ordering::Release);assert_eq!(sys_native_futex(&mut frame),0);frame.rsi=2;assert_eq!(sys_native_futex(&mut frame),2);
 {let mut s=sched::SCHEDULER.lock();*s=fixture(1);let p=&mut s.processes[0];p.clear_futex_wait();p.state=ProcessState::Running;p.linux=Some(Linux::default());}
 WORD.store(0,Ordering::Release);frame.rsi=0;frame.r10=0;frame.rcx=100;assert_eq!(sys_linux_futex(&mut frame),202);assert_eq!(frame.rcx,98);
 for _ in 0..8{frame.rcx=100;assert_eq!(sys_linux_futex(&mut frame),202);assert_eq!(frame.rcx,98);let s=sched::SCHEDULER.lock();assert_eq!(s.processes[0].linux_state().unwrap().futex_wait,Some(4096));}
 {let mut s=sched::SCHEDULER.lock();assert_eq!(s.wake_linux_futex(AddressSpaceIdentity(7),4096,1),1);assert_eq!(s.wake_linux_futex(AddressSpaceIdentity(7),4096,1),0);}
 WORD.store(1,Ordering::Release);assert_eq!(sys_linux_futex(&mut frame),0);assert_eq!(sched::SCHEDULER.lock().processes[0].futex_return_pending,None);
 println!("C11: production syscall mismatch/register/one-shot receipt PASS; VM/trap adapters");
 println!("DELAYED_IPI: native wake and expiry + Linux wake; eight retries each; one registration; no early return PASS");
}
'''
    syscall=(ROOT/'kernel/src/arch/x86_64/syscall.rs').read_text()
    path=a.evidence/'production-admission.rs'
    path.write_text(code.replace('fn main(){','fn lifecycle_controls(){')+admission+function(syscall,'sys_native_futex')+function(syscall,'replay_futex_wait')+function(syscall,'sys_linux_futex'))
    binary=a.evidence/'production-admission'
    subprocess.run(['rustc','--edition=2021',str(path),'-o',str(binary)],check=True,cwd=ROOT)
    subprocess.run([str(binary.resolve())],check=True,cwd=ROOT,timeout=60)

if __name__=='__main__': main()
