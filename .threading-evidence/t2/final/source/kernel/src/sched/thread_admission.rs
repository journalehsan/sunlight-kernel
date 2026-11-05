//! No new lock or counter: the existing scheduler guard owns admission until
//! commit. Finished children remain charged until physical reaping completes.
use super::*;
use crate::process::{foreground::Foreground, thread_admission::*};

pub(crate) struct ThreadSlot {
    index: usize,
    foreground: Foreground,
}

impl Scheduler {
    pub(crate) fn reserve_thread_slot(&mut self) -> Result<ThreadSlot, Error> {
        let parent = self.current_process();
        let identity = parent.address_space.identity();
        if parent.is_exiting()
            || parent.foreground.cancel != 0
            || parent.foreground.forced_exit.is_some()
            || !identity.is_valid()
            || !self.processes.iter().any(|p| {
                p.owns_address_space
                    && p.address_space.identity() == identity
                    && (!p.is_exiting()
                        || (p.state == ProcessState::Finished
                            && p.linux_tgid.is_some()
                            && p.linux_tgid == parent.linux_tgid))
            })
        {
            return Err(Error::Cancelled);
        }
        let mut global = 0usize;
        let mut vm = 0usize;
        for p in &self.processes {
            if !p.owns_address_space && p.state != ProcessState::Reaped {
                global = global.checked_add(1).ok_or(Error::Capacity)?;
                if p.address_space.identity() == identity {
                    vm = vm.checked_add(1).ok_or(Error::Capacity)?;
                }
            }
        }
        check_counts(global, vm, self.processes.len())?;
        // Select precisely one safe slot. Do not invoke the general reaper:
        // reaping may allocate and is not part of construction's transaction.
        let index = self
            .processes
            .iter()
            .enumerate()
            .find(|(idx, p)| {
                p.state == ProcessState::Reaped
                    && p.service_reap_watcher.is_none()
                    && p.owning_core == u8::MAX
                    && p.queued_on_core == u8::MAX
                    && !self.stack_is_pinned(*idx)
                    && (0..self.online_cores).all(|cpu| self.core_current_task(cpu) != Some(*idx))
            })
            .map(|(idx, _)| idx)
            .unwrap_or(self.processes.len());
        if index >= MAX_THREAD_TASK_SLOTS {
            return Err(Error::Capacity);
        }
        let parent = self.current_process();
        // Preflight all inherited allocation sizes, including String headers.
        if parent.env.len() > MAX_ENV_ENTRIES || parent.capabilities.len() > MAX_CAPABILITIES {
            return Err(Error::Capacity);
        }
        let mut bytes = parent
            .capabilities
            .len()
            .checked_mul(core::mem::size_of::<crate::process::Capability>())
            .and_then(|n| n.checked_add(parent.cwd.len()))
            .and_then(|n| {
                n.checked_add(parent.env.len().checked_mul(core::mem::size_of::<(
                    alloc::string::String,
                    alloc::string::String,
                )>())?)
            })
            .ok_or(Error::Capacity)?;
        for (key, value) in parent.env.iter() {
            bytes = bytes
                .checked_add(key.len())
                .and_then(|n| n.checked_add(value.len()))
                .ok_or(Error::Capacity)?;
        }
        if bytes > MAX_INHERITED_BYTES {
            return Err(Error::Capacity);
        }
        // Bound this transaction's peak, including allocator rounding of up
        // to 256 short key/value strings. Storage replacement temporarily owns
        // both old and new buffers, so charge the complete new allocation.
        let mut required = crate::process::KERNEL_STACK_SIZE
            .checked_add(core::mem::size_of::<crate::process::fd_table::FdTable>())
            .and_then(|n| n.checked_add(MAX_INHERITED_BYTES))
            .and_then(|n| n.checked_add(MAX_ENV_ENTRIES * 64 + 1024))
            .ok_or(Error::Capacity)?;
        if self.processes.capacity() < MAX_THREAD_TASK_SLOTS {
            required = required
                .checked_add(
                    MAX_THREAD_TASK_SLOTS
                        .checked_mul(core::mem::size_of::<Process>())
                        .ok_or(Error::Capacity)?,
                )
                .ok_or(Error::Capacity)?;
        }
        for (cpu, state) in CORE_STATES.iter().enumerate() {
            let missing = if SCHEDULER_MODE == SchedulerMode::RoundRobin {
                usize::from(state.lock().rr_queue.capacity() < MAX_THREAD_TASK_SLOTS)
            } else {
                let core = &self.cores[cpu];
                [
                    &core.run_queue_high,
                    &core.run_queue_medium,
                    &core.run_queue_low,
                ]
                .iter()
                .filter(|q| q.capacity() < MAX_THREAD_TASK_SLOTS)
                .count()
            };
            required = required
                .checked_add(
                    missing
                        .checked_mul(MAX_THREAD_TASK_SLOTS * core::mem::size_of::<usize>() + 32)
                        .ok_or(Error::Capacity)?,
                )
                .ok_or(Error::Capacity)?;
        }
        if crate::memory::heap::heap_stats()
            .free
            .checked_sub(required)
            .is_none_or(|free| free < THREAD_HEAP_HEADROOM)
        {
            return Err(Error::NoMemory);
        }
        let mut foreground = parent.foreground;
        foreground.parent_generation = parent.foreground.incarnation;
        foreground.member = parent.foreground.execution();
        foreground.owned = 0;
        foreground.session_owner = false;
        foreground.incarnation = Foreground::allocate_incarnation().ok_or(Error::Capacity)?;
        self.reserve_task_storage()?;
        Ok(ThreadSlot { index, foreground })
    }

    /// Shared with process spawn so process creation cannot grow the table or
    /// queues beyond the bound relied on by admitted workers' later wakeups.
    pub(crate) fn reserve_task_storage(&mut self) -> Result<(), Error> {
        if self.processes.len() >= MAX_THREAD_TASK_SLOTS
            && !self.processes.iter().enumerate().any(|(idx, p)| {
                p.state == ProcessState::Reaped
                    && p.service_reap_watcher.is_none()
                    && p.owning_core == u8::MAX
                    && p.queued_on_core == u8::MAX
                    && !self.stack_is_pinned(idx)
                    && (0..self.online_cores).all(|cpu| self.core_current_task(cpu) != Some(idx))
            })
        {
            return Err(Error::Capacity);
        }
        // Reserve enough for *any* future migration/wakeup of an admitted task.
        // Existing unique queue-membership invariants bound depth by task slots.
        let extra = MAX_THREAD_TASK_SLOTS.saturating_sub(self.processes.len());
        checkpoint("task-table")?;
        self.processes
            .try_reserve_exact(extra)
            .map_err(|_| Error::NoMemory)?;
        for (cpu, state) in CORE_STATES.iter().enumerate() {
            if SCHEDULER_MODE == SchedulerMode::RoundRobin {
                let mut core = state.lock();
                let extra = MAX_THREAD_TASK_SLOTS.saturating_sub(core.rr_queue.len());
                checkpoint("run-queue")?;
                core.rr_queue
                    .try_reserve_exact(extra)
                    .map_err(|_| Error::NoMemory)?;
            } else {
                let core = &mut self.cores[cpu];
                for queue in [
                    &mut core.run_queue_high,
                    &mut core.run_queue_medium,
                    &mut core.run_queue_low,
                ] {
                    checkpoint("run-queue")?;
                    queue
                        .try_reserve_exact(MAX_THREAD_TASK_SLOTS.saturating_sub(queue.len()))
                        .map_err(|_| Error::NoMemory)?;
                }
            }
        }
        Ok(())
    }

    /// Commit: callers have completed every fallible operation and user store.
    /// The same SCHEDULER guard is held from reservation through this insertion.
    /// Snapshot retains, slot insertion and queue admission cannot allocate.
    pub(crate) fn publish_thread(
        &mut self,
        slot: ThreadSlot,
        mut child: Process,
        cpu: usize,
    ) -> usize {
        child.foreground = slot.foreground;
        // Linux's PPID is the group's parent, which can differ from the
        // creating thread. Parent links must match the PID they qualify.
        child.foreground.parent_generation = self
            .processes
            .iter()
            .find(|p| p.pid == child.ppid)
            .map_or(0, |p| p.foreground.incarnation);
        child.fd_table.retain_thread_snapshot();
        let index = slot.index;
        if index == self.processes.len() {
            self.processes.push(child);
        } else {
            debug_assert_eq!(self.processes[index].state, ProcessState::Reaped);
            if self.processes[index].native_thread {
                BORROWER_SLOTS_REUSED.fetch_add(1, Ordering::Relaxed);
            }
            self.processes[index] = child;
        }
        PROCESS_CREATED.fetch_add(1, Ordering::Relaxed);
        self.enqueue_ready_on_cpu(index, cpu);
        index
    }
}
