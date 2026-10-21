"""Host-only audit probe of the production RR selector; requires rustc.

Run: python3 tools/scheduler_policy_probe.py
Extracts production functions into a single-CPU fixture with two Ready tasks.
Ownership, context validation, stealing and full dispatch are stubbed. This is
policy evidence, not a guest benchmark or a proof of full scheduler correctness.
Runs the frozen Phase A selector to preserve the starvation reproduction, then
tests the current selector for eventual service over all supported nice values.
No kernel sources or boot artifacts are changed.
"""
from pathlib import Path
import re
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
src = (root / 'kernel/src/sched/mod.rs').read_text()
def function(name):
    start = src.index('fn ' + name + '(')
    brace = src.index('{', start)
    depth = 1
    end = brace + 1
    while depth:
        depth += (src[end] == '{') - (src[end] == '}')
        end += 1
    return src[start:end]

constants = '\n'.join(re.findall(r'pub const (?:BURST_\w+|MINIMUM_AGED_BURST_SCORE|AGING_THRESHOLD_TICKS|MAX_CREDIT|MAX_DEBT|PROMOTE_LIMIT|SKIP_LIMIT|DECAY_RATE|STARVATION_BOOST|QUANTUM_MIN|QUANTUM_MAX):[^;]+;', src))
fixture = r'''
#![allow(dead_code)]
use std::{collections::VecDeque, sync::{Mutex, MutexGuard}};
struct Lock<T>(Mutex<T>);
impl<T> Lock<T> { fn lock(&self) -> MutexGuard<'_, T> { self.0.lock().unwrap() } }
struct Core { rr_queue: VecDeque<usize>, empty_pops: u64, stale_pops: u64 }
static CORE_STATES: [Lock<Core>; 1] = [Lock(Mutex::new(Core {
    rr_queue: VecDeque::new(), empty_pops: 0, stale_pops: 0
}))];
#[derive(Clone, Copy)] enum ProcessState { Ready }
enum BurstReason { EarlyBlock, FullQuantum, Aged }
struct Process { state: ProcessState, queued_on_core: u8, cpu_mask: u64,
    last_run_tick: u64, aging_boosted_this_pick: bool, counter: i32,
    nice: i8, slice_credit: Option<()>, quantum_override: Option<u32>, burst_score: u32,
    interactive_bonus: i32, aging_counter: u32, stack_handoff_core: u8 }
struct Scheduler { processes: Vec<Process>, global_tick: u64 }
impl Scheduler {
    fn is_live_on_other_core(&self, _: usize, _: usize) -> bool { false }
    fn context_is_dispatchable(&self, _: usize) -> bool { true }
    fn steal_work(&mut self, _: usize) -> Option<usize> { None }
    fn enqueue_process_once(&mut self, _: usize) { panic!("unexpected stale path") }
'''
main = r'''
}
fn main() {
    for burst in [0, 256, 1024] {
    for nice in -10..=10 {
        let p = Process { state: ProcessState::Ready, queued_on_core: 0,
            cpu_mask: 1, last_run_tick: 0, aging_boosted_this_pick: false,
            counter: 0, nice, slice_credit: None, quantum_override: None, burst_score: burst,
            interactive_bonus: 0, aging_counter: 0, stack_handoff_core: u8::MAX };
        let peer = Process { nice: 0, ..p };
        let mut s = Scheduler { processes: vec![p, peer], global_tick: 0 };
        CORE_STATES[0].lock().rr_queue = VecDeque::from([0, 1]);
        let mut selected = [0; 2];
        let mut longest_wait = 0;
        for tick in 1..=10000 {
            s.global_tick = tick;
            if let Some(idx) = s.pick_next_round_robin(0) {
                selected[idx] += 1;
                if idx == 0 { longest_wait = longest_wait.max(tick - s.processes[0].last_run_tick); }
                s.processes[idx].last_run_tick = tick;
                let budget = quantum_ticks(&s.processes[idx]);
                on_task_ran(&mut s.processes[idx], 50, budget);
                s.processes[idx].queued_on_core = 0;
                CORE_STATES[0].lock().rr_queue.push_back(idx);
            }
        }
        // Include the still-Ready censored tail, including never-selected tasks.
        longest_wait = longest_wait.max(10000 - s.processes[0].last_run_tick);
        if burst == 256 && [0, 7, 8, 9, 10].contains(&nice) {
            println!("legacy={} nice={} selected={} peer_selected={} final_counter={} longest_wait={}",
                LEGACY, nice, selected[0], selected[1], s.processes[0].counter, longest_wait);
        }
        if LEGACY && nice >= 8 { assert_eq!(selected[0], 0); }
        else { assert!(selected[0] > 0); }
        if !LEGACY { assert!(longest_wait <= AGING_THRESHOLD_TICKS + 2); }
        assert!(selected[1] > 0);
        if nice == 0 { assert_eq!(selected, [5000, 5000]); }
    }
    }
    for score in [0, 256, 1024] {
        println!("burst={} nice=0 quantum_ticks={}", score, calculate_quantum_with_nice(score, 0));
    }
    if !LEGACY {
        let p = Process { state: ProcessState::Ready, queued_on_core: 0,
            cpu_mask: 1, last_run_tick: 0, aging_boosted_this_pick: false,
            counter: 0, nice: 0, slice_credit: None, quantum_override: None, burst_score: 256,
            interactive_bonus: 0, aging_counter: 0, stack_handoff_core: 1 };
        let peer = Process { stack_handoff_core: u8::MAX, ..p };
        let mut s = Scheduler { processes: vec![p, peer], global_tick: 0 };
        CORE_STATES[0].lock().rr_queue = VecDeque::from([0, 1]);
        STACK_HANDOFFS.pin(1, 0);
        assert_eq!(s.pick_next_round_robin(0), Some(1));
        assert_eq!(CORE_STATES[0].lock().rr_queue.iter().filter(|&&i| i == 0).count(), 1);
        assert!(s.stack_is_pinned(0));
        STACK_HANDOFFS.slot(1).store(handoff::EMPTY, std::sync::atomic::Ordering::Release);
        assert_eq!(s.pick_next_round_robin(0), Some(0));
        println!("remote handoff selector defers without losing queue entry: PASS");
    }
}
'''
for legacy in [True, False]:
    selector = ((root / 'tools/tests/scheduler_rr_phase_a.rs').read_text()
                if legacy else function('pick_next_round_robin'))
    code = fixture + function('stack_is_pinned') + function('stack_is_pinned_remotely') + function('wait_for_pin_release') + selector + main + constants
    code += '\nconst MAX_CORES: usize = 2;\nstatic STACK_HANDOFFS: handoff::Handoffs<MAX_CORES> = handoff::Handoffs::new();\n'
    code += f'#[path = "{root / "kernel/src/sched/handoff.rs"}"] mod handoff;\n'
    code += '\nconst LEGACY: bool = ' + str(legacy).lower() + ';\n'
    code += '\n' + '\n'.join(function(n) for n in [
        'update_burst_score', 'calculate_quantum_with_nice', 'quantum_ticks',
        'accumulate_counter', 'on_task_ran'])
    with tempfile.TemporaryDirectory(prefix='scheduler-policy-') as directory:
        source = Path(directory) / 'probe.rs'
        binary = Path(directory) / 'probe'
        source.write_text(code)
        subprocess.run(['rustc', '--edition=2021', str(source), '-o', str(binary)], check=True)
        subprocess.run([str(binary)], check=True)
