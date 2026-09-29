//! Single producer/consumer mailbox: expensive preparation and retirement stay
//! off the display thread. One pending result bounds memory during rapid edits.
use super::*;
use alloc::boxed::Box;
use core::sync::atomic::{AtomicPtr, AtomicU64};

static SIZE: AtomicU64 = AtomicU64::new(0);
static READY: AtomicPtr<Prepared> = AtomicPtr::new(core::ptr::null_mut());
static RETIRED: AtomicPtr<Prepared> = AtomicPtr::new(core::ptr::null_mut());

pub(super) struct Prepared {
    pub image: Option<Wallpaper>,
    pub config: DesktopConfig,
    pub error: bool,
    pub overrides: Vec<IconOverride>,
}

pub(super) fn start() -> bool {
    // No borrowed argument; mailboxes live for the process lifetime.
    if unsafe { libc::thread::spawn(run, core::ptr::null_mut()) }.is_err() {
        debug_log("[VORTEX] wallpaper worker unavailable\n");
        false
    } else {
        true
    }
}

pub(super) fn take(width: u32, height: u32) -> Option<Box<Prepared>> {
    if width == 0 || height == 0 {
        return None;
    }
    SIZE.store(((width as u64) << 32) | height as u64, Ordering::Release);
    if !RETIRED.load(Ordering::Acquire).is_null() {
        return None;
    }
    let ptr = READY.swap(core::ptr::null_mut(), Ordering::AcqRel);
    if ptr.is_null() {
        None
    } else {
        // Acquire transfers sole ownership from the worker to this UI thread.
        Some(unsafe { Box::from_raw(ptr) })
    }
}

pub(super) fn retire(previous: Box<Prepared>) {
    RETIRED.store(Box::into_raw(previous), Ordering::Release);
}

pub(super) fn pending() -> bool {
    !READY.load(Ordering::Acquire).is_null()
}

extern "C" fn run(_: *mut u8) -> *mut u8 {
    let wait = sunlight_ipc::endpoint_create();
    let mut previous = None;
    let mut previous_size = 0;
    let mut previous_overrides = Vec::new();
    let mut awaiting_retirement = false;
    let mut spare = None;
    // Opt-in native regression: six complete worker/UI ownership round trips,
    // then restore the real config. Never writes the user's desktop settings.
    let regression = option_env!("SUNLIGHT_WALLPAPER_RELOAD_TEST").is_some();
    let mut regression_step = 0u8;
    loop {
        let retired = RETIRED.swap(core::ptr::null_mut(), Ordering::AcqRel);
        if !retired.is_null() {
            unsafe {
                let mut retired = Box::from_raw(retired);
                spare = retired.image.take().or(spare);
            }
            awaiting_retirement = false;
            if regression && regression_step < 6 {
                regression_step += 1;
            }
        }
        let size = SIZE.load(Ordering::Acquire);
        if size != 0 && !awaiting_retirement {
            let mut config = load_desktop_config();
            if regression && regression_step < 6 {
                config.wallpaper = String::from(if regression_step % 2 == 0 {
                    "/var/sunlightos/wallpapers/wallpaper1.simg"
                } else {
                    "/var/sunlightos/wallpapers/wallpaper2.simg"
                });
            }
            let overrides = load_desktop_icon_overrides();
            if previous.as_ref() != Some(&config)
                || size != previous_size
                || previous_overrides != overrides
            {
                // Retry only after another config/size change on allocation
                // failure, avoiding repeated decode work under memory pressure.
                previous = Some(config.clone());
                previous_size = size;
                previous_overrides = overrides.clone();
                let started = monotonic_millis();
                let (decoded, error) = load_wallpaper_from_config(&config);
                let loaded = monotonic_millis();
                let width = (size >> 32) as u32;
                let height = size as u32;
                let image = if let Some(source) = decoded {
                    match Wallpaper::prepare(
                        &source.pixels,
                        source.width,
                        source.height,
                        width,
                        height,
                        spare.take(),
                    ) {
                        Some(image) => Some(image),
                        None => {
                            debug_log("[VORTEX] wallpaper mapping failed; keeping current image\n");
                            let _ = sunlight_ipc::ipc_recv_timeout(wait, 500);
                            continue;
                        }
                    }
                } else {
                    None
                };
                if regression {
                    debug_log(&alloc::format!(
                        "[WALLPAPER-TEST] step={} width={} height={} pixels={}\n",
                        regression_step,
                        width,
                        height,
                        image.as_ref().map(|i| i.pixels().len()).unwrap_or(0)
                    ));
                }
                let prepared = Box::new(Prepared {
                    image,
                    config,
                    error,
                    overrides,
                });
                awaiting_retirement = true;
                READY.store(Box::into_raw(prepared), Ordering::Release);
                if option_env!("SUNLIGHT_UI_AUDIO_DIAGNOSTICS").is_some() {
                    debug_log(&alloc::format!(
                        "[WALLPAPER] load_decode_ms={} scale_ms={}\n",
                        loaded - started,
                        monotonic_millis() - loaded
                    ));
                }
            }
        }
        // Blocking wait, never a spin or a redraw-driven filesystem poll.
        let _ = sunlight_ipc::ipc_recv_timeout(wait, 500);
    }
}
