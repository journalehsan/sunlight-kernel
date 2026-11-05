// Historical Phase A selector: d2faa88e6. Reproduction fixture, never compiled into the kernel.
    pub fn pick_next_round_robin(&mut self, cpu_id: usize) -> Option<usize> {
        let mut remaining = CORE_STATES[cpu_id].lock().rr_queue.len();
        while remaining > 0 {
            let idx = {
                let mut core = CORE_STATES[cpu_id].lock();
                match core.rr_queue.pop_front() {
                    Some(idx) => idx,
                    None => {
                        core.empty_pops += 1;
                        return self.steal_work(cpu_id);
                    }
                }
            };
            remaining -= 1;

            if idx >= self.processes.len() {
                CORE_STATES[cpu_id].lock().stale_pops += 1;
                continue;
            }

            if self.processes[idx].queued_on_core == cpu_id as u8 {
                self.processes[idx].queued_on_core = u8::MAX;
            }

            if !matches!(self.processes[idx].state, ProcessState::Ready)
                || self.is_live_on_other_core(idx, cpu_id)
                || self.processes[idx].cpu_mask & (1u64 << cpu_id) == 0
                || !self.context_is_dispatchable(idx)
            {
                CORE_STATES[cpu_id].lock().stale_pops += 1;
                if matches!(self.processes[idx].state, ProcessState::Ready)
                    && !self.is_live_on_other_core(idx, cpu_id)
                {
                    self.enqueue_process_once(idx);
                }
                continue;
            }

            // Starvation rescue BEFORE accumulate_counter.
            let ticks_waiting = self
                .global_tick
                .saturating_sub(self.processes[idx].last_run_tick);
            if ticks_waiting > AGING_THRESHOLD_TICKS && !self.processes[idx].aging_boosted_this_pick
            {
                self.processes[idx].counter =
                    (self.processes[idx].counter + STARVATION_BOOST).min(MAX_CREDIT);
                self.processes[idx].aging_boosted_this_pick = true;
                update_burst_score(&mut self.processes[idx], BurstReason::Aged);
            }

            accumulate_counter(&mut self.processes[idx]);

            // High priority promotion.
            if self.processes[idx].nice < 0
                && self.processes[idx].counter >= PROMOTE_LIMIT
                && self.processes[idx].quantum_override.is_none()
            {
                self.processes[idx].counter =
                    0_i32.max(self.processes[idx].counter - PROMOTE_LIMIT);
                let base_q = calculate_quantum_with_nice(
                    self.processes[idx].burst_score,
                    self.processes[idx].nice,
                );
                let quantum_override_value = ((base_q * 110) / 100).min(QUANTUM_MAX);
                self.processes[idx].quantum_override = Some(quantum_override_value);
                self.processes[idx].aging_boosted_this_pick = false;
                return Some(idx);
            }

            // Low priority skip.
            if self.processes[idx].nice > 0 && self.processes[idx].counter <= SKIP_LIMIT {
                self.processes[idx].counter += DECAY_RATE;
                self.processes[idx].aging_boosted_this_pick = false;
                self.processes[idx].queued_on_core = cpu_id as u8;
                CORE_STATES[cpu_id].lock().rr_queue.push_back(idx);
                continue;
            }

            self.processes[idx].aging_boosted_this_pick = false;
            return Some(idx);
        }

        self.steal_work(cpu_id)
    }
