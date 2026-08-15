//! Control Panel Sound page. Presentation only; audiod owns policy.

use core::fmt::Write;

use sun_font::{self, FontRole, TextStyle, Typography};
use sunlight_audio::SystemSound;
use sunlight_audiod::{
    map_page_state, AudioClient, AudioClientError, SoundPageView, DEFAULT_TONE_HZ, DEFAULT_TONE_MS,
};
use sunlight_ipc::monotonic_millis;
use sunlight_ui::{
    widgets::{Button, Checkbox, Slider},
    Canvas, Event, MaterialPalette, Point, Rect, Theme,
};

use crate::sysinfo::FixedStr;

const REFRESH_INTERVAL_MS: u64 = 750;
const RETRY_BACKOFF_MS: [u64; 5] = [1_000, 2_000, 5_000, 10_000, 30_000];
const KEY_ESC: u8 = 0x01;
const KEY_LEFT: u8 = 0x4b;
const KEY_RIGHT: u8 = 0x4d;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SoundAction {
    None,
    Back,
}

pub struct SoundPageState {
    view: SoundPageView,
    slider: Slider,
    system_toggle: Checkbox<'static>,
    system_slider: Slider,
    last_sent: Option<u8>,
    last_system_sent: Option<u8>,
    last_drag_ms: u64,
    last_system_drag_ms: u64,
    status: FixedStr<80>,
    status_until_ms: u64,
    next_refresh_ms: u64,
    failures: usize,
}

impl SoundPageState {
    pub fn new() -> Self {
        let view = map_page_state(Err(AudioClientError::ServiceUnavailable));
        Self {
            view,
            slider: Slider::horizontal(Rect::default())
                .with_range(0, 100)
                .with_value(view.volume as u32),
            system_toggle: Checkbox::new(Rect::default(), "Play system sounds")
                .with_font(&Typography::UI_REGULAR),
            system_slider: Slider::horizontal(Rect::default())
                .with_range(0, 100)
                .with_value(view.system_sounds_volume as u32),
            last_sent: None,
            last_system_sent: None,
            last_drag_ms: 0,
            last_system_drag_ms: 0,
            status: FixedStr::empty(),
            status_until_ms: 0,
            next_refresh_ms: 0,
            failures: 0,
        }
    }

    pub fn refresh(&mut self) -> bool {
        let now = monotonic_millis();
        match AudioClient::new().snapshot() {
            Ok(snapshot) => {
                self.view = map_page_state(Ok(snapshot));
                if !self.slider.dragging {
                    self.slider.set_value(self.view.volume as u32);
                }
                self.system_toggle.checked = self.view.system_sounds_enabled;
                if !self.system_slider.dragging {
                    self.system_slider
                        .set_value(self.view.system_sounds_volume as u32);
                }
                self.failures = 0;
                self.next_refresh_ms = now.saturating_add(REFRESH_INTERVAL_MS);
                if now >= self.status_until_ms {
                    self.status.clear();
                }
            }
            Err(error) => {
                self.view = map_page_state(Err(error));
                let step = RETRY_BACKOFF_MS[self.failures.min(RETRY_BACKOFF_MS.len() - 1)];
                self.failures = self.failures.saturating_add(1);
                self.next_refresh_ms = now.saturating_add(step);
                self.status.set(error_label(error));
            }
        }
        true
    }

    pub fn refresh_due(&self) -> bool {
        monotonic_millis() >= self.next_refresh_ms
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme, win_w: u32, win_h: u32) {
        canvas.clear_transparent(Rect::new(0, 0, win_w, win_h));
        let materials = MaterialPalette::new(theme);
        let header = Rect::new(0, 0, win_w, 44);
        canvas.fill_material(header, materials.card_glass.with_radius(0).without_border());
        canvas.draw_rect(Rect::new(0, 43, win_w, 1), theme.chrome.subtle_border);
        draw_text(canvas, "Sound", 18, 10, 24, FontRole::UiTitle, theme.text);

        Button::secondary(back_rect(win_h), "Back")
            .with_font(&Typography::UI_MEDIUM)
            .draw(canvas, theme);
        Button::secondary(refresh_rect(win_w, win_h), "Refresh")
            .with_font(&Typography::UI_MEDIUM)
            .draw(canvas, theme);

        let card = Rect::new(14, 54, win_w.saturating_sub(28), 100);
        canvas.fill_material(card, materials.card_glass.with_radius(7).without_border());
        canvas.stroke_rounded_rect(card, 7, 1, theme.chrome.subtle_border);
        draw_text(
            canvas,
            "Output",
            card.x + 12,
            card.y + 10,
            18,
            FontRole::UiMedium,
            theme.text,
        );
        draw_text(
            canvas,
            self.view.device_name,
            card.x + 12,
            card.y + 34,
            20,
            FontRole::UiRegular,
            if self.view.available {
                theme.text
            } else {
                theme.text_dim
            },
        );
        draw_text(
            canvas,
            self.view.state_label,
            card.x + 12,
            card.y + 54,
            18,
            FontRole::UiSmall,
            if self.view.available {
                theme.ok
            } else {
                theme.warn
            },
        );
        if let Some(fmt) = self.view.format {
            let mut line = FixedStr::<40>::empty();
            let _ = write!(
                &mut line,
                "{} Hz, {}-bit stereo",
                fmt.sample_rate_hz, fmt.bits_per_sample
            );
            draw_text(
                canvas,
                line.as_str(),
                card.x + 12,
                card.y + 74,
                16,
                FontRole::UiSmall,
                theme.text_dim,
            );
        }

        let vol = Rect::new(14, 164, win_w.saturating_sub(28), 76);
        canvas.fill_material(vol, materials.card_glass.with_radius(7).without_border());
        canvas.stroke_rounded_rect(vol, 7, 1, theme.chrome.subtle_border);
        draw_text(
            canvas,
            "Volume",
            vol.x + 12,
            vol.y + 10,
            18,
            FontRole::UiMedium,
            theme.text,
        );
        let mute_rect = Rect::new(vol.x + 12, vol.y + 36, 72, 28);
        Button::secondary(mute_rect, if self.view.muted { "Unmute" } else { "Mute" })
            .with_font(&Typography::UI_SMALL)
            .draw(canvas, theme);
        let slider = Slider::horizontal(Rect::new(
            mute_rect.right() + 10,
            vol.y + 38,
            vol.w.saturating_sub(160),
            24,
        ))
        .with_range(0, 100)
        .with_value(self.slider.value);
        slider.draw(canvas, theme);
        let mut pct = FixedStr::<8>::empty();
        let _ = write!(&mut pct, "{}%", self.slider.value);
        draw_text(
            canvas,
            pct.as_str(),
            vol.right() - 48,
            vol.y + 38,
            24,
            FontRole::UiMedium,
            theme.text,
        );

        let system_card = system_card_rect(win_w);
        canvas.fill_material(
            system_card,
            materials.card_glass.with_radius(7).without_border(),
        );
        canvas.stroke_rounded_rect(system_card, 7, 1, theme.chrome.subtle_border);
        draw_text(
            canvas,
            "System Sounds",
            system_card.x + 12,
            system_card.y + 8,
            18,
            FontRole::UiMedium,
            theme.text,
        );
        let mut toggle = Checkbox::new(system_toggle_rect(win_w), "Play system sounds")
            .with_font(&Typography::UI_REGULAR);
        toggle.checked = self.system_toggle.checked;
        toggle.active = self.system_toggle.active;
        toggle.draw(canvas, theme);
        draw_text(
            canvas,
            "Event volume",
            system_card.x + 12,
            system_card.y + 54,
            16,
            FontRole::UiSmall,
            theme.text_dim,
        );
        let sys_slider = Slider::horizontal(system_slider_rect(win_w))
            .with_range(0, 100)
            .with_value(self.system_slider.value);
        sys_slider.draw(canvas, theme);
        let mut sys_pct = FixedStr::<8>::empty();
        let _ = write!(&mut sys_pct, "{}%", self.system_slider.value);
        draw_text(
            canvas,
            sys_pct.as_str(),
            system_card.right() - 48,
            system_card.y + 52,
            22,
            FontRole::UiMedium,
            theme.text,
        );
        draw_text(
            canvas,
            "Preview",
            system_card.x + 12,
            system_card.y + 88,
            16,
            FontRole::UiSmall,
            theme.text_dim,
        );
        for (index, sound) in PREVIEW_SOUNDS.iter().copied().enumerate() {
            Button::secondary(preview_button_rect(win_w, index), sound.label())
                .with_font(&Typography::UI_SMALL)
                .draw(canvas, theme);
        }

        let test_card = test_card_rect(win_w);
        canvas.fill_material(
            test_card,
            materials.card_glass.with_radius(7).without_border(),
        );
        canvas.stroke_rounded_rect(test_card, 7, 1, theme.chrome.subtle_border);
        draw_text(
            canvas,
            "Test",
            test_card.x + 12,
            test_card.y + 10,
            18,
            FontRole::UiMedium,
            theme.text,
        );
        draw_text(
            canvas,
            "Device output test tone.",
            test_card.x + 12,
            test_card.y + 32,
            16,
            FontRole::UiSmall,
            theme.text_dim,
        );
        let test = test_button_rect(win_w);
        if self.view.available {
            Button::new(test, "Test Sound")
                .with_font(&Typography::UI_MEDIUM)
                .draw(canvas, theme);
        } else {
            Button::secondary(test, "Test Sound")
                .with_font(&Typography::UI_MEDIUM)
                .draw(canvas, theme);
        }

        if !self.status.is_empty() {
            let color = if self.view.available && !self.status.as_str().contains("unavailable") {
                theme.text_dim
            } else {
                theme.warn
            };
            draw_text(
                canvas,
                self.status.as_str(),
                18,
                win_h as i32 - 72,
                16,
                FontRole::UiSmall,
                color,
            );
        }
    }

    pub fn update(&mut self, event: Event, win_w: u32, win_h: u32) -> SoundAction {
        if matches!(event, Event::Tick) && self.refresh_due() {
            let _ = self.refresh();
            return SoundAction::None;
        }
        match event {
            Event::KeyPress {
                keycode: KEY_ESC,
                pressed: true,
                ..
            } => return SoundAction::Back,
            Event::Click { x, y } => {
                let pt = Point::new(x, y);
                if back_rect(win_h).contains(pt) {
                    return SoundAction::Back;
                }
                if refresh_rect(win_w, win_h).contains(pt) {
                    let _ = self.refresh();
                    return SoundAction::None;
                }
                if mute_hit(win_w).contains(pt) && self.view.available {
                    let _ = AudioClient::new().set_mute(!self.view.muted);
                    let _ = self.refresh();
                    return SoundAction::None;
                }
                if test_button_rect(win_w).contains(pt) {
                    self.play_test_sound();
                    return SoundAction::None;
                }
                for (index, sound) in PREVIEW_SOUNDS.iter().copied().enumerate() {
                    if preview_button_rect(win_w, index).contains(pt) {
                        self.preview_system_sound(sound);
                        return SoundAction::None;
                    }
                }
            }
            Event::KeyPress {
                keycode: KEY_LEFT,
                pressed: true,
                ..
            } if self.view.available => {
                let next = self.slider.value.saturating_sub(5);
                self.slider.set_value(next);
                self.send_volume(next as u8, true);
            }
            Event::KeyPress {
                keycode: KEY_RIGHT,
                pressed: true,
                ..
            } if self.view.available => {
                let next = (self.slider.value + 5).min(100);
                self.slider.set_value(next);
                self.send_volume(next as u8, true);
            }
            _ => {}
        }

        let slider_rect = slider_hit(win_w);
        self.slider.rect = slider_rect;
        let was_dragging = self.slider.dragging;
        if self.view.available && self.slider.update(event) {
            let now = monotonic_millis();
            let value = self.slider.value as u8;
            if self.slider.dragging {
                if self.last_sent != Some(value) && now.saturating_sub(self.last_drag_ms) >= 30 {
                    self.send_volume(value, false);
                    self.last_drag_ms = now;
                }
            } else if was_dragging {
                self.send_volume(value, true);
            } else if self.last_sent != Some(value) {
                self.send_volume(value, true);
            }
        }

        self.system_toggle.rect = system_toggle_rect(win_w);
        let previous_toggle = self.system_toggle.checked;
        if self.view.available && self.system_toggle.update(event) {
            if self.system_toggle.checked != previous_toggle {
                self.set_system_sounds_enabled(self.system_toggle.checked);
            }
        }

        self.system_slider.rect = system_slider_rect(win_w);
        let was_system_dragging = self.system_slider.dragging;
        if self.view.available && self.system_slider.update(event) {
            let now = monotonic_millis();
            let value = self.system_slider.value as u8;
            if self.system_slider.dragging {
                if self.last_system_sent != Some(value)
                    && now.saturating_sub(self.last_system_drag_ms) >= 30
                {
                    self.send_system_volume(value, false);
                    self.last_system_drag_ms = now;
                }
            } else if was_system_dragging {
                self.send_system_volume(value, true);
            } else if self.last_system_sent != Some(value) {
                self.send_system_volume(value, true);
            }
        }
        SoundAction::None
    }

    fn play_test_sound(&mut self) {
        if !self.view.available {
            self.set_status(error_label(AudioClientError::Unavailable));
            return;
        }
        if self.view.muted || self.view.volume == 0 {
            self.set_status("Unmute and raise volume to hear the test");
        }
        match AudioClient::new().play_test_sound() {
            Ok(()) => {
                if !self.view.muted && self.view.volume > 0 {
                    let mut line = FixedStr::<80>::empty();
                    let _ = write!(
                        &mut line,
                        "Playing {} Hz test tone ({} ms)",
                        DEFAULT_TONE_HZ, DEFAULT_TONE_MS
                    );
                    self.set_status(line.as_str());
                }
            }
            Err(err) => self.set_status(error_label(err)),
        }
    }

    fn send_volume(&mut self, value: u8, preview: bool) {
        let client = AudioClient::new();
        if client.set_volume(value).is_err() {
            self.set_status("Could not change volume");
            return;
        }
        self.last_sent = Some(value);
        self.view.volume = value;
        if preview {
            self.play_volume_preview();
        }
    }

    fn play_volume_preview(&mut self) {
        if !self.view.available || self.view.muted || self.view.volume == 0 {
            return;
        }
        let _ = AudioClient::new().play_volume_preview();
    }

    fn set_system_sounds_enabled(&mut self, enabled: bool) {
        match AudioClient::new().set_system_sounds_enabled(enabled) {
            Ok(snapshot) => {
                self.view = map_page_state(Ok(snapshot));
                self.system_toggle.checked = enabled;
                self.set_status(if enabled {
                    "System sounds enabled"
                } else {
                    "System sounds disabled"
                });
            }
            Err(err) => {
                self.system_toggle.checked = self.view.system_sounds_enabled;
                self.set_status(error_label(err));
            }
        }
    }

    fn send_system_volume(&mut self, value: u8, preview: bool) {
        match AudioClient::new().set_system_sounds_volume(value) {
            Ok(snapshot) => {
                self.view = map_page_state(Ok(snapshot));
                self.last_system_sent = Some(value);
                if preview {
                    self.preview_system_sound(SystemSound::VolumeChanged);
                }
            }
            Err(err) => self.set_status(error_label(err)),
        }
    }

    fn preview_system_sound(&mut self, sound: SystemSound) {
        if !self.view.available {
            self.set_status(error_label(AudioClientError::Unavailable));
            return;
        }
        if self.view.muted || self.view.volume == 0 || self.system_slider.value == 0 {
            self.set_status("Raise master and system volume to hear previews");
            return;
        }
        match AudioClient::new().preview_system_sound(sound) {
            Ok(()) => {
                let mut line = FixedStr::<80>::empty();
                let _ = write!(&mut line, "Previewing {}", sound.label());
                self.set_status(line.as_str());
            }
            Err(err) => self.set_status(error_label(err)),
        }
    }

    fn set_status(&mut self, text: &str) {
        self.status.set(text);
        self.status_until_ms = monotonic_millis().saturating_add(2_500);
    }
}

fn error_label(error: AudioClientError) -> &'static str {
    match error {
        AudioClientError::ServiceUnavailable => "Audio service unavailable",
        AudioClientError::Timeout => "Audio service timed out",
        AudioClientError::Unavailable => "No audio output device available",
        AudioClientError::DeviceFailed => "Audio device failed",
        _ => "Unable to read audio state",
    }
}

fn back_rect(win_h: u32) -> Rect {
    Rect::new(14, win_h as i32 - 44, 72, 28)
}

fn refresh_rect(win_w: u32, win_h: u32) -> Rect {
    Rect::new(win_w as i32 - 96, win_h as i32 - 44, 80, 28)
}

fn test_card_rect(win_w: u32) -> Rect {
    Rect::new(14, 436, win_w.saturating_sub(28), 62)
}

fn test_button_rect(win_w: u32) -> Rect {
    Rect::new(win_w as i32 - 160, 451, 132, 32)
}

fn mute_hit(_win_w: u32) -> Rect {
    Rect::new(26, 200, 72, 28)
}

fn slider_hit(win_w: u32) -> Rect {
    Rect::new(108, 202, win_w.saturating_sub(176), 24)
}

const PREVIEW_SOUNDS: [SystemSound; 5] = [
    SystemSound::Notification,
    SystemSound::Success,
    SystemSound::Warning,
    SystemSound::Error,
    SystemSound::Critical,
];

fn system_card_rect(win_w: u32) -> Rect {
    Rect::new(14, 250, win_w.saturating_sub(28), 176)
}

fn system_toggle_rect(win_w: u32) -> Rect {
    Rect::new(26, 280, win_w.saturating_sub(52), 22)
}

fn system_slider_rect(win_w: u32) -> Rect {
    Rect::new(112, 308, win_w.saturating_sub(180), 24)
}

fn preview_button_rect(win_w: u32, index: usize) -> Rect {
    let left = 26;
    let gap = 5;
    let available = win_w.saturating_sub(52);
    let width = available.saturating_sub(gap * 4) / 5;
    Rect::new(
        left + index as i32 * (width as i32 + gap as i32),
        380,
        width,
        28,
    )
}

fn draw_text(
    canvas: &mut Canvas,
    text: &str,
    x: i32,
    y: i32,
    h: u32,
    role: FontRole,
    color: sunlight_ui::Color,
) {
    sun_font::draw_text_vcenter(canvas, text, x, y, h, &TextStyle::new(role, color));
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunlight_audio::{AudioDeviceState, OutputDeviceKind};
    use sunlight_audiod::{map_page_state, AudioSnapshot};

    #[test]
    fn maps_ready_device() {
        let view = map_page_state(Ok(AudioSnapshot {
            service_generation: 1,
            state: AudioDeviceState::Ready,
            volume: 68,
            muted: false,
            last_nonzero: 68,
            kind: OutputDeviceKind::QemuHdAudio,
            sample_rate_hz: 48_000,
            channels: 2,
            bits: 16,
            underruns: 0,
            frames_played: 10,
            vendor_id: 0x8086,
            device_id: 0x2668,
            system_sounds_enabled: true,
            system_sounds_volume: 60,
            system_sound_queue_len: 0,
        }));
        assert_eq!(view.device_name, "QEMU HD Audio");
        assert_eq!(view.state_label, "Ready");
        assert_eq!(view.volume, 68);
        assert!(view.system_sounds_enabled);
        assert_eq!(view.system_sounds_volume, 60);
        assert!(view.available);
    }

    #[test]
    fn maps_unavailable_and_mute() {
        let missing = map_page_state(Err(AudioClientError::ServiceUnavailable));
        assert!(missing.service_missing);
        assert_eq!(missing.device_name, "Audio service unavailable");
        let muted = map_page_state(Ok(AudioSnapshot {
            service_generation: 2,
            state: AudioDeviceState::Ready,
            volume: 40,
            muted: true,
            last_nonzero: 40,
            kind: OutputDeviceKind::QemuHdAudio,
            sample_rate_hz: 48_000,
            channels: 2,
            bits: 16,
            underruns: 0,
            frames_played: 0,
            vendor_id: 0,
            device_id: 0,
            system_sounds_enabled: true,
            system_sounds_volume: 60,
            system_sound_queue_len: 0,
        }));
        assert!(muted.muted);
        assert_eq!(muted.icon, sunlight_audio::VolumeIconKind::Off);
    }

    #[test]
    fn test_button_is_inside_test_card() {
        let card = test_card_rect(500);
        let button = test_button_rect(500);
        assert!(card.contains(Point::new(button.x + 4, button.y + 4)));
        assert!(button.w >= 120);
        assert!(button.h >= 28);
    }

    #[test]
    fn system_sound_controls_fit_and_expose_required_previews() {
        let card = system_card_rect(500);
        assert!(card.contains(Point::new(30, 285)));
        for index in 0..PREVIEW_SOUNDS.len() {
            let button = preview_button_rect(500, index);
            assert!(card.contains(Point::new(button.x + 2, button.y + 2)));
        }
        assert_eq!(
            PREVIEW_SOUNDS,
            [
                SystemSound::Notification,
                SystemSound::Success,
                SystemSound::Warning,
                SystemSound::Error,
                SystemSound::Critical,
            ]
        );
    }
}
