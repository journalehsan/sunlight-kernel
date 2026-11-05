//! Immutable xAPIC topology. Logical zero is always the BSP; AP slots are
//! logical_id - 1 in validated enumeration order. CPU hotplug is unsupported.
pub const MAX_CPUS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyError {
    Capacity,
    Duplicate,
    MissingBsp,
    UnsupportedApicId,
    InvalidDestination,
    OfflineDestination,
}

#[derive(Debug)]
pub struct CpuTopology {
    apic_ids: [u32; MAX_CPUS],
    count: usize,
}

impl CpuTopology {
    pub fn new(ids: &[u32], bsp: u32) -> Result<Self, TopologyError> {
        if ids.is_empty() || ids.len() > MAX_CPUS {
            return Err(TopologyError::Capacity);
        }
        let mut result = Self {
            apic_ids: [0; MAX_CPUS],
            count: ids.len(),
        };
        let mut next = 1;
        let mut found = false;
        for (i, &id) in ids.iter().enumerate() {
            // 0xff is the xAPIC broadcast destination, never a unicast CPU.
            if id >= 255 {
                return Err(TopologyError::UnsupportedApicId);
            }
            if ids[..i].contains(&id) {
                return Err(TopologyError::Duplicate);
            }
            if id == bsp {
                result.apic_ids[0] = id;
                found = true;
            } else {
                if next < MAX_CPUS {
                    result.apic_ids[next] = id;
                }
                next += 1;
            }
        }
        if !found {
            return Err(TopologyError::MissingBsp);
        }
        Ok(result)
    }

    pub fn count(&self) -> usize {
        self.count
    }
    pub fn apic_id(&self, logical: usize) -> Option<u32> {
        self.apic_ids.get(..self.count)?.get(logical).copied()
    }
    pub fn logical_id(&self, apic: u32) -> Option<usize> {
        self.apic_ids[..self.count]
            .iter()
            .position(|&id| id == apic)
    }
    pub fn ap_slot(&self, logical: usize) -> Option<usize> {
        if logical > 0 && logical < self.count {
            Some(logical - 1)
        } else {
            None
        }
    }
    pub fn destination(&self, logical: usize, online: u64) -> Result<u32, TopologyError> {
        let id = self
            .apic_id(logical)
            .ok_or(TopologyError::InvalidDestination)?;
        if online & (1u64 << logical) == 0 {
            return Err(TopologyError::OfflineDestination);
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(ids: &[u32], bsp: u32) {
        let map = CpuTopology::new(ids, bsp).unwrap();
        assert_eq!(map.apic_id(0), Some(bsp));
        let mut mask = 0u64;
        for &id in ids {
            let logical = map.logical_id(id).unwrap();
            assert_eq!(map.destination(logical, u64::MAX), Ok(id));
            assert_eq!(mask & (1u64 << logical), 0);
            mask |= 1u64 << logical;
            assert_eq!(map.ap_slot(logical), logical.checked_sub(1));
        }
        assert_eq!(mask.count_ones() as usize, ids.len());
        assert_eq!(map.logical_id(256), None);
        assert_eq!(
            map.destination(ids.len(), u64::MAX),
            Err(TopologyError::InvalidDestination)
        );
        assert_eq!(
            map.destination(0, 0),
            Err(TopologyError::OfflineDestination)
        );
        assert_eq!(map.ap_slot(ids.len()), None);
    }
    #[test]
    fn contiguous() {
        check(&[0, 1, 2, 3], 0);
    }
    #[test]
    fn sparse() {
        check(&[0, 2, 17, 80], 0);
    }
    #[test]
    fn nonzero_bsp() {
        check(&[42, 2, 17, 80], 42);
    }
    #[test]
    fn bsp_later() {
        check(&[2, 17, 42, 80], 42);
    }
    #[test]
    fn high_ids() {
        check(&[254, 64, 128], 128);
    }
    #[test]
    fn maximum() {
        check(&core::array::from_fn::<_, 64, _>(|i| (i * 3) as u32), 93);
    }
    #[test]
    fn invalid_topologies() {
        for (ids, bsp, error) in [
            (&[0, 0][..], 0, TopologyError::Duplicate),
            (&[1, 2], 0, TopologyError::MissingBsp),
            (&[0, 255], 0, TopologyError::UnsupportedApicId),
            (&[0, 256], 0, TopologyError::UnsupportedApicId),
            (&[0, u32::MAX], 0, TopologyError::UnsupportedApicId),
            (&[], 0, TopologyError::Capacity),
        ] {
            assert_eq!(CpuTopology::new(ids, bsp).unwrap_err(), error);
        }
        assert_eq!(
            CpuTopology::new(&[0; 65], 0).unwrap_err(),
            TopologyError::Capacity
        );
    }
}
