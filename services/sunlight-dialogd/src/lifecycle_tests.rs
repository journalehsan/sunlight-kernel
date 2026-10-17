//! Exercise the actual compositor policy with the dialog host's window identity.
#[allow(dead_code)]
#[path = "../../sunlight-display/src/app_lifecycle.rs"]
mod app_lifecycle;

use crate::{color_picker::ColorPicker, WINDOW_TITLE};
use app_lifecycle::{AppAction, AppTracker};
use sunlight_dialogs::{
    color::RecentColors, decode_result, encode_result, ColorPickerRequest, DialogResult,
};
use sunlight_ui::Event;

#[test]
fn closing_picker_keeps_host_alive_to_reply_and_open_another_dialog() {
    let mut tracker = AppTracker::new();
    // Reuse the same service PID across confirmed and cancelled requests.
    for (window_id, confirm) in [(1, true), (2, false), (3, true)] {
        // CREATE_WINDOW carries eight title bytes; CONFIGURE_WINDOW supplies
        // the full identity before the window is shown, just like Window::connect.
        tracker.register_window(25, window_id, &WINDOW_TITLE[..8], false);
        tracker.update_window_title(window_id, WINDOW_TITLE);
        let mut picker = ColorPicker::new(
            ColorPickerRequest {
                title: "Annotation color".into(),
                initial_color: "#302513".into(),
                allow_alpha: false,
            },
            RecentColors::default(),
        );
        picker.update(Event::key_press(
            if confirm { 0x1c } else { 0x01 },
            true,
            false,
            false,
            false,
            false,
        ));
        let result = picker.result().unwrap();

        // Window::run removes the native window BEFORE dialogd encodes/replies.
        // A Terminate action here reproduces the SIGTERM / Err(Corrupt) failure.
        assert!(matches!(
            tracker.unregister_window(window_id, 1000),
            AppAction::None
        ));
        let received = decode_result(&encode_result(result)).unwrap();
        assert_eq!(
            received,
            if confirm {
                DialogResult::ColorSelected("#302513".into())
            } else {
                DialogResult::Cancelled
            }
        );
    }
}
