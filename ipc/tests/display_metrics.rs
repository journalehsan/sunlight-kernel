use sunlight_ipc::{
    validate_size, DisplayMetrics, PixelFormat, ScreenBackend, ScreenRect, MAX_DIM, SCALE_FP_ONE,
};

#[test]
fn safe_fallback_is_valid() {
    let m = DisplayMetrics::safe_fallback();
    assert_eq!(m.width_px, 1280);
    assert_eq!(m.height_px, 800);
    assert_eq!(m.stride_bytes, 1280 * 4);
}

#[test]
fn validate_size_rejects_garbage() {
    assert!(validate_size(0, 800).is_none());
    assert!(validate_size(1280, 0).is_none());
    assert!(validate_size(MAX_DIM + 1, 720).is_none());
    assert_eq!(validate_size(1920, 1080), Some((1920, 1080)));
}

#[test]
fn reply_round_trip_preserves_fields() {
    let m = DisplayMetrics::new(
        1366,
        768,
        1366 * 4,
        PixelFormat::Xrgb8888,
        ScreenBackend::VirtioGpu,
    );
    let words = m.pack_reply_words();
    let decoded = DisplayMetrics::from_reply(&words);
    assert_eq!(decoded.width_px, 1366);
    assert_eq!(decoded.height_px, 768);
    assert_eq!(decoded.stride_bytes, 1366 * 4);
    assert_eq!(decoded.pixel_format, PixelFormat::Xrgb8888);
    assert_eq!(decoded.backend, ScreenBackend::VirtioGpu);
}

#[test]
fn legacy_reply_word0_only() {
    let words = [(1024u64) | ((768u64) << 32)];
    let m = DisplayMetrics::from_reply(&words);
    assert_eq!(m.width_px, 1024);
    assert_eq!(m.height_px, 768);
    assert_eq!(m.stride_bytes, 1024 * 4);
    assert_eq!(m.scale_fp, SCALE_FP_ONE);
}

#[test]
fn mouse_clamp_respects_bounds() {
    let m = DisplayMetrics::new(
        1280,
        720,
        1280 * 4,
        PixelFormat::Xrgb8888,
        ScreenBackend::VirtioGpu,
    );
    assert_eq!(m.clamp_point(2000, 900), (1279, 719));
    assert_eq!(m.clamp_i32_point(-5, 800), (0, 719));
}

#[test]
fn window_origin_stays_on_screen() {
    let m = DisplayMetrics::new(
        1024,
        768,
        1024 * 4,
        PixelFormat::Xrgb8888,
        ScreenBackend::LimineFramebuffer,
    );
    let (x, y) = m.fit_window_origin(900, 700, 50, 800, 700);
    assert!(x + 900 <= 1024);
    assert!(y + 700 <= 768);
    assert!(y >= 50);
}

#[test]
fn initial_placement_fits_multiple_resolutions() {
    for (w, h) in [(1920, 1080), (1366, 768), (1024, 768)] {
        let m = DisplayMetrics::new(w, h, w * 4, PixelFormat::Xrgb8888, ScreenBackend::VirtioGpu);
        let client_w = 600;
        let client_h = 440;
        let chrome_w = client_w + 2;
        let chrome_h = client_h + 34;
        let (x, y) = m.initial_window_origin(3, client_w, client_h, chrome_w, chrome_h, 50);
        assert!(x + chrome_w <= w, "width overflow at {w}x{h}");
        assert!(y + chrome_h <= h, "height overflow at {w}x{h}");
        assert!(y >= 50);
    }
}

#[test]
fn wallpaper_rect_covers_screen() {
    let m = DisplayMetrics::new(
        1600,
        900,
        1600 * 4,
        PixelFormat::Xrgb8888,
        ScreenBackend::VirtioGpu,
    );
    let r = m.wallpaper_target_rect();
    assert_eq!(
        r,
        ScreenRect {
            x: 0,
            y: 0,
            w: 1600,
            h: 900
        }
    );
}

#[test]
fn stride_aware_pixel_offset() {
    let m = DisplayMetrics::new(
        800,
        600,
        832 * 4,
        PixelFormat::Xrgb8888,
        ScreenBackend::LimineFramebuffer,
    );
    assert_eq!(m.pixel_offset(10, 2), 2 * m.stride_words() + 10);
}
