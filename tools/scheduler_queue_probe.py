"""Execute production queue publication helpers against a host request fixture.
This checks the internal/external notification distinction, not full dispatch.
"""
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
source = (root / 'kernel/src/sched/mod.rs').read_text()


def function(name):
    start = source.index('fn ' + name + '(')
    end = source.index('{', start) + 1
    depth = 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]


fixture = r'''
#![allow(dead_code)]
use std::{collections::VecDeque, sync::{Mutex, MutexGuard, atomic::{AtomicBool, AtomicU64, Ordering}}};
struct Lock<T>(Mutex<T>);
impl<T> Lock<T> { fn lock(&self) -> MutexGuard<'_, T> { self.0.lock().unwrap() } }
struct Core { rr_queue: VecDeque<usize>, run_queue_high: VecDeque<usize>,
    run_queue_medium: VecDeque<usize>, run_queue_low: VecDeque<usize>, current_task: Option<usize> }
impl Core { const fn new() -> Self { Self { rr_queue: VecDeque::new(),
    run_queue_high: VecDeque::new(), run_queue_medium: VecDeque::new(),
    run_queue_low: VecDeque::new(), current_task: None } } }
static CORE_STATES: [Lock<Core>; 1] = [Lock(Mutex::new(Core::new()))];
static SCHEDULER_READY: AtomicBool = AtomicBool::new(true);
static DISPATCH_NOTIFICATION_MASK: AtomicU64 = AtomicU64::new(0);
static REQUESTS: requests::RescheduleRequests = requests::RescheduleRequests::new();
#[derive(PartialEq)] enum SchedulerMode { RoundRobin, Bore }
const SCHEDULER_MODE: SchedulerMode = SchedulerMode::RoundRobin;
enum ProcessState { Ready, Running, Finished }
enum QueueTier { High, Medium, Low }
struct Process { state: ProcessState, owning_core: u8, queued_on_core: u8 }
impl Process { fn get_queue_tier(&self) -> QueueTier { QueueTier::Medium } }
fn current_cpu_id() -> usize { 0 }
fn reschedule_bit(cpu: usize) -> u64 { 1 << cpu }
fn request_reschedule_on(cpu: usize) { REQUESTS.request(cpu); }
struct Scheduler { processes: Vec<Process>, cores: [Core; 1] }
impl Scheduler {
    fn core_current_task(&self, c: usize) -> Option<usize> { CORE_STATES[c].lock().current_task }
'''
test = r'''
}
fn main() {
    let ready = || Process { state: ProcessState::Ready, owning_core: u8::MAX, queued_on_core: u8::MAX };
    let mut s = Scheduler { processes: vec![ready(), ready()], cores: [Core::new()] };
    s.enqueue_process_to_core(0, 0);
    assert!(REQUESTS.take(0));
    CORE_STATES[0].lock().rr_queue.clear(); // model dequeue before rotation
    s.processes[0].queued_on_core = u8::MAX;
    s.queue_process_to_core(0, 0, false);
    assert!(!REQUESTS.pending(0), "internal rotation rearmed dispatch");
    assert_eq!(CORE_STATES[0].lock().rr_queue.len(), 1);
    CORE_STATES[0].lock().rr_queue.clear();
    s.processes[0].queued_on_core = u8::MAX;
    s.enqueue_process_to_core(1, 0); // a new wake after entry consumption
    s.queue_process_to_core(0, 0, false);
    assert!(REQUESTS.take(0), "internal rotation lost a new external request");
    assert!(!REQUESTS.take(0)); // duplicate IPI must not cause another dispatch
    CORE_STATES[0].lock().rr_queue.clear();
    s.processes[0].owning_core = 0;
    s.enqueue_process_to_core(0, 0);
    s.processes[1].state = ProcessState::Finished;
    s.enqueue_process_to_core(1, 0);
    assert!(CORE_STATES[0].lock().rr_queue.is_empty());
    assert!(!REQUESTS.pending(0));
    println!("queue publication/internal rotation/request preservation/owned and terminal rejection: PASS");
}
'''
code = fixture + function('enqueue_process_to_core') + function('queue_process_to_core') + test
code += f'#[path = "{root / "kernel/src/sched/requests.rs"}"] mod requests;\n'
with tempfile.TemporaryDirectory(prefix='scheduler-queue-') as directory:
    path = Path(directory)
    (path / 'probe.rs').write_text(code)
    subprocess.run(['rustc', '--edition=2021', str(path / 'probe.rs'), '-o', str(path / 'probe')], check=True)
    subprocess.run([str(path / 'probe')], check=True)
