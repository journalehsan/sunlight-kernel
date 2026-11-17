//! Runs in its own host process, so parallel unit tests cannot obscure leaks.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use sunlight_empires::{Civilization, Command, Envelope, Tile, World};
struct Count;
static LIVE: AtomicIsize = AtomicIsize::new(0);
static BYTES: AtomicIsize = AtomicIsize::new(0);
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            LIVE.fetch_add(1, Ordering::SeqCst);
            BYTES.fetch_add(l.size() as isize, Ordering::SeqCst);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
        BYTES.fetch_sub(l.size() as isize, Ordering::SeqCst);
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOC: Count = Count;
#[test]
fn no_growth_across_scene_lifetimes() {
    let baseline = (LIVE.load(Ordering::SeqCst), BYTES.load(Ordering::SeqCst));
    for iteration in 0..128 {
        {
            let mut world = if iteration%2==0 {World::new_match(Civilization::Persia)} else {World::new(Civilization::Persia)};
            world
                .submit(Envelope {
                    tick: 0,
                    player: 0,
                    sequence: 0,
                    command: Command::AssignWorker {
                        worker: 2,
                        node: Tile::new(14, 8),
                    },
                })
                .unwrap();
            for _ in 0..400 {
                world.step();
            }
            let save = sunlight_empires::replay::save(&world);
            let restored = sunlight_empires::replay::load(&save).unwrap();
            assert_eq!(world.checksum(), restored.checksum());
        }
        assert_eq!(
            (LIVE.load(Ordering::SeqCst), BYTES.load(Ordering::SeqCst)),
            baseline
        );
    }
}
