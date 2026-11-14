//! Hosted diagnostics only. No timing enters the simulator or disk format.
use std::{hint::black_box, time::Instant};
use sunlight_sunfs::{block::*, bootstrap::Bootstrap, codec::*, geometry::*};
use sunlight_sunfs_check::{check, Bytes, Status};
use sunlight_sunfs_lab::simulator::Simulator;
fn main() {
    let g = Geometry::calculate(Parameters::laboratory(8 * 1024 * 1024, 256)).unwrap();
    let p = Bootstrap::plan(g, [1; 16]).unwrap();
    let page = &p.dependencies[0];
    let bytes = page.encode().unwrap();
    let iterations = 10000;
    let t = Instant::now();
    for _ in 0..iterations {
        black_box(page.encode().unwrap());
    }
    let encode = t.elapsed().as_nanos() / iterations;
    let t = Instant::now();
    for _ in 0..iterations {
        black_box(Page::decode(black_box(&bytes), page.block).unwrap());
    }
    let decode = t.elapsed().as_nanos() / iterations;
    println!("{{\"kind\":\"codec\",\"iterations\":{iterations},\"encode_ns_per_page\":{encode},\"decode_ns_per_page\":{decode}}}");
    for mib in [8, 16, 64] {
        let g = Geometry::calculate(Parameters::laboratory(mib * 1024 * 1024, 256)).unwrap();
        let t = Instant::now();
        let p = Bootstrap::plan(g, [1; 16]).unwrap();
        let plan_ns = t.elapsed().as_nanos();
        let mut s = Simulator::new((mib * 1024 * 1024) as usize, 0x534653).unwrap();
        let mut peak = s.memory_bytes();
        for page in p.superblocks.iter().chain(&p.dependencies) {
            s.write(page.block * 4096, &page.encode().unwrap()).unwrap();
        }
        peak = peak.max(s.memory_bytes());
        let mut times = Vec::new();
        for _ in 0..5 {
            let mut sim = Simulator::new((mib * 1024 * 1024) as usize, 0x534653).unwrap();
            let t = Instant::now();
            p.publish(&mut sim).unwrap();
            times.push(t.elapsed().as_nanos());
        }
        times.sort();
        p.publish(&mut s).unwrap();
        let t = Instant::now();
        for _ in 0..20 {
            let result = check(&mut Bytes(s.durable()));
            assert_eq!(result.status, Status::Clean);
            black_box(result);
        }
        let check_ns = t.elapsed().as_nanos() / 20;
        println!("{{\"kind\":\"hosted\",\"volume_mib\":{mib},\"plan_ns\":{plan_ns},\"format_median_ns\":{},\"checker_mean_ns\":{check_ns},\"simulator_buffer_peak_bytes\":{peak},\"metadata_blocks\":{}}}",times[2],g.bootstrap_blocks);
    }
    for bps in [64, 256, 512] {
        for mib in [1, 2, 4, 8, 16, 64, 256, 1024, 4096] {
            match Geometry::calculate(Parameters::laboratory(mib*1024*1024,bps)){
            Ok(g)=>println!("{{\"kind\":\"geometry\",\"volume_mib\":{mib},\"segment_blocks\":{bps},\"segments\":{},\"segment_table_pages\":{},\"bootstrap_blocks\":{},\"summary_pages\":{},\"metadata_bound_blocks\":{},\"cleaner_reserve_blocks\":{},\"transaction_bound_blocks\":{},\"minimum_blocks_for_this_table\":{},\"reserved_fixed_blocks\":{},\"tail_bytes\":{},\"union_bitmap_bytes_per_snapshot\":{},\"full_global_table_rewrite_bytes_at_node_cap\":{}}}",
                g.segment_count,g.segment_table_pages,g.bootstrap_blocks,g.summary_pages,g.metadata_bound,g.cleaner_reserve,g.transaction_bound,g.minimum_blocks,g.segment_start,g.tail_bytes,g.segment_count*bps as u64/8,
                (3+g.maximum_nodes.div_ceil(82)+g.maximum_orphans.div_ceil(124)+g.segment_table_pages)*4096),
            Err(e)=>println!("{{\"kind\":\"geometry\",\"volume_mib\":{mib},\"segment_blocks\":{bps},\"rejected\":\"{e:?}\"}}"),
        }
        }
    }
    for bps in [64, 256, 512] {
        for blocks in (bps as u64 * 2..bps as u64 * 100).step_by(bps as usize) {
            if let Ok(g) = Geometry::calculate(Parameters::laboratory(blocks * 4096, bps)) {
                println!("{{\"kind\":\"minimum\",\"segment_blocks\":{bps},\"volume_blocks\":{blocks},\"volume_bytes\":{},\"metadata_blocks\":{},\"cleaner_reserve_blocks\":{}}}",blocks*4096,g.bootstrap_blocks,g.cleaner_reserve);
                break;
            }
        }
    }
}
