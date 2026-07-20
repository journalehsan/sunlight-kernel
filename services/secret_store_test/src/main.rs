#![no_std]
#![no_main]

use sunlight_ipc::debug_log;
use sunlight_libc::secret_store::{
    wipe, CreateResult, SecretFileOptions, SecretPublishMode, SecretStore,
};

const TEST_PATH: &[u8] = b"/etc/sunlight/secret-store-test.key";
const TEST_SIZE: usize = 48;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

fn valid(bytes: &[u8]) -> bool {
    bytes.len() == TEST_SIZE && bytes[0] == 0xa5 && (bytes[1] == 1 || bytes[1] == 2)
}

fn fill(secret: &mut [u8], version: u8) -> bool {
    secret[0] = 0xa5;
    secret[1] = version;
    sunlight_libc::getrandom(&mut secret[2..], 0) == (secret.len() - 2) as isize
}

#[no_mangle]
extern "C" fn _start() -> ! {
    let mut store = SecretStore::new();
    let mut options = SecretFileOptions::system(TEST_PATH);
    let mut candidate = [0u8; TEST_SIZE];
    let mut loaded = [0u8; TEST_SIZE];
    let _ = store.cleanup_stale_temps(options);

    if !fill(&mut candidate, 1) {
        debug_log("[SECRET-TEST] secure randomness unavailable");
        sunlight_libc::exit(1);
    }

    match store.create_if_absent(options, &mut candidate, valid) {
        Ok(CreateResult::Created) => debug_log("[SECRET-TEST] create published"),
        Ok(CreateResult::Existing) => debug_log("[SECRET-TEST] existing retained"),
        Err(_) => {
            debug_log("[SECRET-TEST] create failed");
            sunlight_libc::exit(1);
        }
    }

    let first_len = match store.load(options, &mut loaded, valid) {
        Ok(bytes) => bytes.len(),
        Err(_) => {
            debug_log("[SECRET-TEST] load failed");
            sunlight_libc::exit(1);
        }
    };
    if first_len != TEST_SIZE {
        debug_log("[SECRET-TEST] invalid length");
        sunlight_libc::exit(1);
    }
    wipe(&mut loaded);

    if !fill(&mut candidate, 2) {
        debug_log("[SECRET-TEST] secure randomness unavailable");
        sunlight_libc::exit(1);
    }
    options.publish_mode = SecretPublishMode::ReplaceExisting;
    if store.replace(options, &mut candidate, valid).is_err() {
        debug_log("[SECRET-TEST] replace failed");
        sunlight_libc::exit(1);
    }
    if store.load(options, &mut loaded, valid).is_err() || loaded[1] != 2 {
        debug_log("[SECRET-TEST] replacement reload failed");
        sunlight_libc::exit(1);
    }
    wipe(&mut loaded);
    debug_log("[SECRET-TEST] create-load-replace OK");
    sunlight_libc::exit(0);
}
