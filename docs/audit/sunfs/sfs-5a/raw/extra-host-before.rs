mod pci {
    pub unsafe fn inl(_: u16) -> u32 { 0 }
    pub unsafe fn inw(_: u16) -> u16 { 0 }
    pub unsafe fn outb(_: u16, _: u8) {}
    pub unsafe fn outl(_: u16, _: u32) {}
    pub unsafe fn outw(_: u16, _: u16) {}
}
#[allow(dead_code)]
mod blk {
    include!("/home/ehsantor/Projects/sunlightos-kernel/docs/audit/sunfs/sfs-5a/raw/blk-before-host-shim.rs");
    #[cfg(test)]
    mod extra {
        use super::*;
        fn device(q: &mut [u64; 2048], r: &mut [u64; 512]) -> VirtioBlk {
            let q = q.as_mut_ptr() as u64;
            let r = r.as_mut_ptr() as u64;
            VirtioBlk { io_base: 0, queue_size: 8, desc_virt:q, avail_virt:q+128,
                used_virt:q+4096, req_phys:r, req_virt:r, avail_idx:0,last_used_idx:0,
                capacity_sectors:8192,flush_supported:false,read_only:false }
        }
        #[test]
        fn unsupported_flush_does_not_submit_or_acknowledge() {
            let mut q=[0;2048]; let mut r=[0;512]; let mut d=device(&mut q,&mut r);
            assert!(!unsafe { d.flush() });
            assert_eq!(d.avail_idx,0);
            assert!(q.iter().chain(r.iter()).all(|v|*v==0));
        }
        #[test]
        fn unsolicited_used_entry_poisoned_without_submission() {
            let mut q=[0;2048]; let mut r=[0;512]; let mut d=device(&mut q,&mut r);
            unsafe { ((d.used_virt+2) as *mut u16).write_volatile(2); }
            let old_q=q; let old_r=r;
            assert!(!unsafe { d.read_block(0,&mut [0;512]) });
            assert_eq!(d.avail_idx,0);
            assert!(q==old_q && r==old_r);
        }
    }
}
