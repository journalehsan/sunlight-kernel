
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
fn clear_futex_wait(&mut self) {
        self.complete_futex_wait(0);
        self.native_futex_result = None;
        self.futex_return_pending = None;
    }
fn complete_futex_wait(&mut self, result: u64) {
        #[cfg(all(debug_assertions, feature = "atomic_futex_test"))]
        crate::sched::t4a_fixture::completing(self, result);
        if let Some(address) = self.native_futex_wait.take() {
            self.native_futex_result = Some((address, result));
        }
        self.native_futex_deadline = None;
        if let Some(state) = self.linux_state_mut() {
            state.futex_wait = None;
        }
    }
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
fn wake_native_futex(
        &mut self,
        identity: crate::process::mm2b_state::AddressSpaceIdentity,
        address: u64,
        limit: usize,
    ) -> usize {
        let mut woken = 0;
        for idx in 0..self.processes.len() {
            if woken == limit {
                break;
            }
            if self.processes[idx].state == ProcessState::BlockedOnIo
                && self.processes[idx].address_space.identity() == identity
                && self.processes[idx].native_futex_wait == Some(address)
            {
                self.wake_native_futex_index(idx);
                woken += 1;
            }
        }
        woken
    }
fn wake_native_futex_index(&mut self, idx: usize) {
        #[cfg(all(debug_assertions, feature = "atomic_futex_test"))]
        t4a_fixture::waking(self, idx);
        self.record_wakeup(idx, false);
        self.processes[idx].state = ProcessState::Ready;
        self.processes[idx].priority_wake_pending = true;
        self.remove_from_ready_queues(idx);
        if let Some(cpu) = self.live_owner_core(idx) {
            request_reschedule_on(cpu);
        } else {
            self.enqueue_ready(idx);
        }
        #[cfg(all(debug_assertions, feature = "thread_migration_test"))]
        t3r_fixture::woken(self, idx);
    }
fn wake_linux_futex(
        &mut self,
        identity: crate::process::mm2b_state::AddressSpaceIdentity,
        address: u64,
        limit: usize,
    ) -> usize {
        let mut woken = 0;
        for idx in 0..self.processes.len() {
            if woken == limit {
                break;
            }
            if self.processes[idx].state != ProcessState::BlockedOnIo
                || self.processes[idx].address_space.identity() != identity
                || self.processes[idx].linux_state().and_then(|s| s.futex_wait) != Some(address)
            {
                continue;
            }
            self.wake_linux_futex_index(idx);
            woken += 1;
        }
        woken
    }
fn wake_linux_futex_index(&mut self, idx: usize) {
        #[cfg(all(debug_assertions, feature = "atomic_futex_test"))]
        t4a_fixture::waking(self, idx);
        self.record_wakeup(idx, false);
        self.processes[idx].state = ProcessState::Ready;
        self.processes[idx].priority_wake_pending = true;
        self.remove_from_ready_queues(idx);
        if let Some(cpu) = self.live_owner_core(idx) {
            request_reschedule_on(cpu);
        } else {
            self.enqueue_ready(idx);
        }
    }
fn expire_native_futex_timeouts(&mut self) {
        let now = crate::timekeeping::monotonic_ns();
        for idx in 0..self.processes.len() {
            if self.processes[idx].state == ProcessState::BlockedOnIo
                && self.processes[idx]
                    .native_futex_deadline
                    .is_some_and(|deadline| deadline <= now)
            {
                self.processes[idx].complete_futex_wait(2);
                self.wake_native_futex_index(idx);
            }
        }
    }
fn record_wakeup(&mut self, idx: usize, ipc: bool) {
        // Generic signal/I/O wakes must detach too: otherwise a later I/O
        // block can be consumed by an old futex wake or expired deadline.
        self.processes[idx].complete_futex_wait(0);
        #[cfg(all(debug_assertions, feature = "fastlane_measurement_test"))]
        self.processes[idx].measurement.wakeup();
        #[cfg(feature = "cpu_accounting_diag")]
        {
            let audit = &mut self.processes[idx].wake_audit;
            audit[0] = audit[0].saturating_add(1);
            if ipc {
                audit[1] = audit[1].saturating_add(1);
            }
            let c = &mut self.cpu_events[current_cpu_id()];
            c[1] = c[1].saturating_add(1);
            if ipc {
                c[2] = c[2].saturating_add(1);
            }
        }
        #[cfg(not(feature = "cpu_accounting_diag"))]
        let _ = (idx, ipc);
    }
fn wake_for_signal(&mut self, pid: usize) {
        let Some(idx) = self.process_index_by_pid(pid) else {
            return;
        };
        #[cfg(all(debug_assertions, feature = "native_doorbell_test"))]
        self.doorbell_spurious_wake(idx);
        #[cfg(all(debug_assertions, feature = "fastlane_doorbell_test"))]
        self.fastlane_bell_spurious(idx);
        match self.processes[idx].state {
            ProcessState::BlockedOnIpc
            | ProcessState::BlockedOnTimer
            | ProcessState::BlockedOnIo
            | ProcessState::Suspended => {
                self.processes[idx].tty_write_wait = None;
                self.processes[idx].tty_read_wait = None;
                if let Some(state) = self.processes[idx].linux_state_mut() {
                    state.poll_wake_tick = None;
                }
                self.record_wakeup(idx, self.processes[idx].state == ProcessState::BlockedOnIpc);
                self.processes[idx].state = ProcessState::Ready;
                self.processes[idx].priority_wake_pending = true;
                self.processes[idx].block_start_tick = self.global_tick;
                self.remove_from_ready_queues(idx);
                if let Some(cpu_id) = self.live_owner_core(idx) {
                    request_reschedule_on(cpu_id);
                } else {
                    self.enqueue_ready(idx);
                }
            }
            _ => {}
        }
    }
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
