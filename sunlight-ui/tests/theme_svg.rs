#![cfg(feature = "svg")]
use sunlight_ui::{
    image::{decode_svg, IconImage},
    Canvas, Color, Rect,
};
const ICONS: &[(&str, &[u8])] = &[
    (
        "actions/16/document-new.svg",
        include_bytes!("../../docs/icons/SunlightOS/actions/16/document-new.svg"),
    ),
    (
        "actions/16/edit-find-symbolic.svg",
        include_bytes!("../../docs/icons/SunlightOS/actions/16/edit-find-symbolic.svg"),
    ),
    (
        "actions/16/folder-new.svg",
        include_bytes!("../../docs/icons/SunlightOS/actions/16/folder-new.svg"),
    ),
    (
        "actions/16/sort-name.svg",
        include_bytes!("../../docs/icons/SunlightOS/actions/16/sort-name.svg"),
    ),
    (
        "actions/16/system-lock-screen-symbolic.svg",
        include_bytes!("../../docs/icons/SunlightOS/actions/16/system-lock-screen-symbolic.svg"),
    ),
    (
        "actions/16/system-reboot-symbolic.svg",
        include_bytes!("../../docs/icons/SunlightOS/actions/16/system-reboot-symbolic.svg"),
    ),
    (
        "actions/16/system-shutdown-symbolic.svg",
        include_bytes!("../../docs/icons/SunlightOS/actions/16/system-shutdown-symbolic.svg"),
    ),
    (
        "actions/16/view-refresh.svg",
        include_bytes!("../../docs/icons/SunlightOS/actions/16/view-refresh.svg"),
    ),
    (
        "actions/32/system-suspend-symbolic.svg",
        include_bytes!("../../docs/icons/SunlightOS/actions/32/system-suspend-symbolic.svg"),
    ),
    (
        "actions/scalable/xsi-user-trash-symbolic.svg",
        include_bytes!("../../docs/icons/SunlightOS/actions/scalable/xsi-user-trash-symbolic.svg"),
    ),
    (
        "actions/scalable/xsi-utilities-terminal-symbolic.svg",
        include_bytes!(
            "../../docs/icons/SunlightOS/actions/scalable/xsi-utilities-terminal-symbolic.svg"
        ),
    ),
    (
        "apps/48/about.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/about.svg"),
    ),
    (
        "apps/48/accessories-calculator.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/accessories-calculator.svg"),
    ),
    (
        "apps/48/apifox.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/apifox.svg"),
    ),
    (
        "apps/48/applications-system.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/applications-system.svg"),
    ),
    (
        "apps/48/bomber.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/bomber.svg"),
    ),
    (
        "apps/48/cpu-x.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/cpu-x.svg"),
    ),
    (
        "apps/48/galculator.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/galculator.svg"),
    ),
    (
        "apps/48/hwinfo.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/hwinfo.svg"),
    ),
    (
        "apps/48/internet-web-browser.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/internet-web-browser.svg"),
    ),
    (
        "apps/48/kate.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/kate.svg"),
    ),
    (
        "apps/48/ksysguard.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/ksysguard.svg"),
    ),
    (
        "apps/48/libreoffice-writer.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/libreoffice-writer.svg"),
    ),
    (
        "apps/48/mupdf.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/mupdf.svg"),
    ),
    (
        "apps/48/org.kde.dolphin.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/org.kde.dolphin.svg"),
    ),
    (
        "apps/48/org.kde.merkuro.calendar.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/org.kde.merkuro.calendar.svg"),
    ),
    (
        "apps/48/preferences-desktop-wallpaper.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/preferences-desktop-wallpaper.svg"),
    ),
    (
        "apps/48/preferences-system-session-services.svg",
        include_bytes!(
            "../../docs/icons/SunlightOS/apps/48/preferences-system-session-services.svg"
        ),
    ),
    (
        "apps/48/preferences-system-time.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/preferences-system-time.svg"),
    ),
    (
        "apps/48/preferences-system.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/preferences-system.svg"),
    ),
    (
        "apps/48/system-run.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/system-run.svg"),
    ),
    (
        "apps/48/utilities-terminal.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/48/utilities-terminal.svg"),
    ),
    (
        "apps/symbolic/clock-app-symbolic.svg",
        include_bytes!("../../docs/icons/SunlightOS/apps/symbolic/clock-app-symbolic.svg"),
    ),
    (
        "devices/16/audio-input-microphone.svg",
        include_bytes!("../../docs/icons/SunlightOS/devices/16/audio-input-microphone.svg"),
    ),
    (
        "devices/64/audio-card.svg",
        include_bytes!("../../docs/icons/SunlightOS/devices/64/audio-card.svg"),
    ),
    (
        "devices/64/battery.svg",
        include_bytes!("../../docs/icons/SunlightOS/devices/64/battery.svg"),
    ),
    (
        "devices/64/computer.svg",
        include_bytes!("../../docs/icons/SunlightOS/devices/64/computer.svg"),
    ),
    (
        "devices/64/drive-harddisk.svg",
        include_bytes!("../../docs/icons/SunlightOS/devices/64/drive-harddisk.svg"),
    ),
    (
        "devices/64/input-mouse.svg",
        include_bytes!("../../docs/icons/SunlightOS/devices/64/input-mouse.svg"),
    ),
    (
        "devices/64/network-card.svg",
        include_bytes!("../../docs/icons/SunlightOS/devices/64/network-card.svg"),
    ),
    (
        "devices/64/video-display.svg",
        include_bytes!("../../docs/icons/SunlightOS/devices/64/video-display.svg"),
    ),
    (
        "mimetypes/32/application-json.svg",
        include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/application-json.svg"),
    ),
    (
        "mimetypes/32/application-octet-stream.svg",
        include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/application-octet-stream.svg"),
    ),
    (
        "mimetypes/32/application-x-executable.svg",
        include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/application-x-executable.svg"),
    ),
    (
        "mimetypes/32/audio-x-generic.svg",
        include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/audio-x-generic.svg"),
    ),
    (
        "mimetypes/32/image-x-generic.svg",
        include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/image-x-generic.svg"),
    ),
    (
        "mimetypes/32/inode-directory.svg",
        include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/inode-directory.svg"),
    ),
    (
        "mimetypes/32/text-markdown.svg",
        include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/text-markdown.svg"),
    ),
    (
        "mimetypes/32/text-plain.svg",
        include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/text-plain.svg"),
    ),
    (
        "mimetypes/32/text-rust.svg",
        include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/text-rust.svg"),
    ),
    (
        "mimetypes/32/text-x-generic.svg",
        include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/text-x-generic.svg"),
    ),
    (
        "mimetypes/32/unknown.svg",
        include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/unknown.svg"),
    ),
    (
        "mimetypes/32/video-x-generic.svg",
        include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/video-x-generic.svg"),
    ),
    (
        "places/16/folder-desktop.svg",
        include_bytes!("../../docs/icons/SunlightOS/places/16/folder-desktop.svg"),
    ),
    (
        "places/16/folder-documents.svg",
        include_bytes!("../../docs/icons/SunlightOS/places/16/folder-documents.svg"),
    ),
    (
        "places/16/folder-downloads.svg",
        include_bytes!("../../docs/icons/SunlightOS/places/16/folder-downloads.svg"),
    ),
    (
        "places/16/folder-music.svg",
        include_bytes!("../../docs/icons/SunlightOS/places/16/folder-music.svg"),
    ),
    (
        "places/16/folder-network.svg",
        include_bytes!("../../docs/icons/SunlightOS/places/16/folder-network.svg"),
    ),
    (
        "places/16/folder-pictures.svg",
        include_bytes!("../../docs/icons/SunlightOS/places/16/folder-pictures.svg"),
    ),
    (
        "places/16/folder-videos.svg",
        include_bytes!("../../docs/icons/SunlightOS/places/16/folder-videos.svg"),
    ),
    (
        "places/16/folder.svg",
        include_bytes!("../../docs/icons/SunlightOS/places/16/folder.svg"),
    ),
    (
        "places/16/system-file-manager.svg",
        include_bytes!("../../docs/icons/SunlightOS/places/16/system-file-manager.svg"),
    ),
    (
        "places/16/user-home.svg",
        include_bytes!("../../docs/icons/SunlightOS/places/16/user-home.svg"),
    ),
    (
        "places/16/user-trash.svg",
        include_bytes!("../../docs/icons/SunlightOS/places/16/user-trash.svg"),
    ),
    (
        "places/symbolic/user-trash-full-symbolic.svg",
        include_bytes!("../../docs/icons/SunlightOS/places/symbolic/user-trash-full-symbolic.svg"),
    ),
    (
        "preferences/22/yast-nfs-server.svg",
        include_bytes!("../../docs/icons/SunlightOS/preferences/22/yast-nfs-server.svg"),
    ),
];
#[test]
fn every_used_theme_icon_renders_at_menu_and_desktop_sizes() {
    for (name, bytes) in ICONS {
        let icon = IconImage::parse(bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        for size in [16, 32, 48, 64] {
            let image = icon.rasterize(size, size).unwrap();
            assert_eq!((image.width, image.height), (size, size), "{name}");
            assert!(
                (0..size).any(|y| (0..size).any(|x| image.pixel_argb(x, y) >> 24 != 0)),
                "empty icon: {name} at {size}px"
            );
        }
    }
}
const RED: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect x="4" y="4" width="8" height="8" fill="#ff0000" fill-opacity="0.5"/></svg>"##;
#[test]
fn svg_alpha_reaches_the_canvas_without_double_premultiplication() {
    let icon = IconImage::parse(RED).unwrap();
    let raster = icon.rasterize(16, 16).unwrap();
    assert_eq!(raster.pixel_argb(0, 0), 0);
    assert_eq!(raster.pixel_argb(8, 8), 0x80ff0000);
    let mut pixels = [0xff0000ff; 16 * 16];
    let mut canvas = Canvas::new(&mut pixels, 16, 16, 16);
    canvas.draw_icon(&icon, Rect::new(0, 0, 16, 16));
    assert_eq!(pixels[0], 0xff0000ff);
    assert_eq!(pixels[8 * 16 + 8], 0xff80007f);
}
#[test]
fn file_svg_and_embedded_svg_share_raster_results() {
    let embedded = IconImage::parse(RED).unwrap().rasterize(32, 32).unwrap();
    let owned = decode_svg(RED, 32, 32).unwrap();
    for y in 0..32 {
        for x in 0..32 {
            assert_eq!(embedded.pixel_argb(x, y), owned.pixel(x, y));
        }
    }
}
#[test]
fn svg_preserves_aspect_ratio_and_rejects_invalid_or_unbounded_inputs() {
    const WIDE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="8"><rect width="16" height="8" fill="red"/></svg>"##;
    let image = IconImage::parse(WIDE).unwrap().rasterize(16, 16).unwrap();
    assert_eq!(image.pixel_argb(8, 0), 0);
    assert_eq!(image.pixel_argb(8, 8), Color::rgb(255, 0, 0).0);
    assert!(IconImage::parse(b"<svg broken").is_err());
    assert!(decode_svg(RED, 0, 16).is_err());
    assert!(decode_svg(&vec![b' '; 2 * 1024 * 1024 + 1], 16, 16).is_err());
}
