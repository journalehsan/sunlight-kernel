use sunlight_ui::{
    app::{take_close_requested, App, Window, WindowConfig, WindowDecoration},
    Theme,
};
#[cfg(all(feature = "native-std", target_os = "sunlightos"))]
extern crate sunlight_libc;

fn main() -> anyhow::Result<()> {
    let document_path = document_argument(std::env::args_os().skip(1));
    #[cfg(feature = "native-ui-smoke")]
    let path = native_smoke::prepare()?;
    let mut window = Window::connect(WindowConfig {
        title: "Document Viewer",
        width: 800,
        height: 700,
        decoration: WindowDecoration::Normal,
    })
    .ok_or_else(|| anyhow::anyhow!("SunlightOS display service is unavailable"))?;
    sunlight_ipc::debug_log("[DOCVIEW] window ready\n");
    let theme = Theme::sunlight_dark();
    #[cfg(feature = "native-ui-smoke")]
    let _ = document_path;
    #[cfg(not(feature = "native-ui-smoke"))]
    let path = match document_path {
        Some(path) => path,
        None => {
            let mut canvas = window.canvas();
            let bounds = sunlight_ui::Rect::new(0, 0, canvas.width, canvas.height);
            canvas.fill_rect(bounds, theme.bg);
            window.commit();
            match choose_document()? {
                Some(path) => path,
                None => return Ok(()),
            }
        }
    };
    let mut app = sunlight_docview::ui::DocumentApp::new(path.into())?;
    app.view(&mut window.canvas(), &theme);
    window.commit();
    let mut title = String::new();
    #[cfg(feature = "native-ui-smoke")]
    let mut smoke = native_smoke::Navigation::default();
    while !take_close_requested() {
        let event = window.poll_event();
        let mut redraw = app.update(event);
        if let Some(event) = window.take_window_event() {
            redraw |= app.window_event(event);
        }
        #[cfg(feature = "native-ui-smoke")]
        if smoke.advance(&mut app)? {
            anyhow::ensure!(window.activate(), "activate viewer window failed");
            app.view(&mut window.canvas(), &theme);
            window.commit();
            sunlight_ipc::debug_log("[DOCVIEW] native window navigation/zoom OK\n");
            // Hold the actual window until the boot harness has captured it.
            loop {
                if matches!(window.poll_event(), sunlight_ui::Event::Key('q')) {
                    break;
                }
            }
            break;
        }
        let next_title = app.title();
        if next_title != title && window.set_title(&next_title) {
            title = next_title;
        }
        if redraw {
            app.view(&mut window.canvas(), &theme);
            window.commit();
        }
    }
    app.shutdown()?;
    #[cfg(feature = "native-ui-smoke")]
    sunlight_ipc::debug_log("[DOCVIEW] native close OK\n");
    Ok(())
}

/// Launchers append their trace metadata after any document argument.
fn document_argument(
    arguments: impl IntoIterator<Item = std::ffi::OsString>,
) -> Option<std::ffi::OsString> {
    let mut path = None;
    for argument in arguments {
        if let Some(trace) =
            sunlight_ipc::launch_trace::parse_launch_arg(argument.as_encoded_bytes())
        {
            sunlight_ipc::launch_trace::set_current(trace);
        } else if !argument
            .as_encoded_bytes()
            .starts_with(b"--sunlight-launch=")
            && path.is_none()
        {
            path = Some(argument);
        }
    }
    path
}

#[cfg(not(feature = "native-ui-smoke"))]
fn choose_document() -> anyhow::Result<Option<std::ffi::OsString>> {
    use sunlight_dialogs::{DialogClient, DialogRequest, DialogResult, OpenFileRequest};
    let request = DialogRequest::OpenFile(OpenFileRequest {
        title: "Open Document".into(),
        initial_dir: None,
        allowed_mime_types: Vec::new(),
        allowed_extensions: ["pdf", "ps", "eps", "xps", "cbz", "epub"]
            .into_iter()
            .map(String::from)
            .collect(),
        allow_multiple: false,
        show_preview: true,
        confirm_button_label: Some("Open".into()),
    });
    sunlight_ipc::debug_log("[DOCVIEW] Open Document dialog requested\n");
    match DialogClient::new()
        .show_native(&request)
        .map_err(|error| anyhow::anyhow!("Open Document dialog failed: {error:?}"))?
    {
        DialogResult::FileSelected(path) => Ok(Some(path.into())),
        DialogResult::Cancel | DialogResult::Cancelled | DialogResult::Dismissed => Ok(None),
        DialogResult::Error(error) => anyhow::bail!("{error}"),
        _ => anyhow::bail!("unexpected Open Document dialog response"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_metadata_is_not_a_document_path() {
        use sunlight_ipc::launch_trace::{self, LaunchSource, LaunchTrace};
        let trace = std::ffi::OsString::from("--sunlight-launch=42:shell:100");
        assert_eq!(document_argument([trace.clone()]), None);
        assert_eq!(
            launch_trace::current(),
            Some(LaunchTrace::new(42, LaunchSource::Shell, 100))
        );
        let path = std::ffi::OsString::from("/root/Documents/Test PDF.PDF");
        assert_eq!(document_argument([path.clone(), trace]), Some(path));
        launch_trace::clear_current();
    }
}

#[cfg(feature = "native-ui-smoke")]
mod native_smoke {
    use super::*;
    use std::{
        ffi::{CString, OsString},
        time::{Duration, Instant},
    };
    use sunlight_ui::Event;

    pub fn prepare() -> anyhow::Result<OsString> {
        let path = "/tmp/docview-fixture.pdf";
        std::fs::write(
            path,
            include_bytes!("../../sunlight-docserv/tests/fixtures/sample.pdf"),
        )?;
        if sunlight_ipc::nameserver_lookup_timeout("display_server", 100).is_none() {
            spawn("/sbin/sunlight-display")?;
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        while sunlight_ipc::nameserver_lookup_timeout("display_server", 100).is_none() {
            anyhow::ensure!(
                Instant::now() < deadline,
                "display service did not register"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        // Ordinary windows stay behind the secure login surface until the
        // authenticated desktop session has completed activation.
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if let Some(session) =
                sunlight_ipc::nameserver_lookup_timeout(sunlight_ipc::SESSION_ENDPOINT, 100)
            {
                if let Ok(reply) = sunlight_ipc::ipc_call_timeout(
                    session,
                    sunlight_ipc::IpcMsg::with_label(
                        sunlight_ipc::SessionMsg::SESSION_CURRENT_IDENTITY,
                    ),
                    500,
                ) {
                    if reply.label == sunlight_ipc::SessionMsg::REPLY {
                        break;
                    }
                }
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "desktop session did not activate"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        spawn("/bin/sunlight-files")?;
        Ok(path.into())
    }

    fn spawn(path: &str) -> anyhow::Result<()> {
        let executable = CString::new(path)?;
        let args = [executable.as_ptr(), core::ptr::null()];
        let result: u64;
        // Test-only direct native spawn avoids unsupported Linux fork.
        unsafe {
            core::arch::asm!("syscall",
            inlateout("rax") if cfg!(target_os = "sunlightos") { 39 } else { sunlight_ipc::NATIVE_SYSCALL_TAG | 39 } => result,
            inlateout("rdi") executable.as_ptr() as u64 => _,
            inlateout("rsi") args.as_ptr() as u64 => _,
            inlateout("rdx") u64::MAX => _, lateout("rcx") _, lateout("r11") _);
        }
        anyhow::ensure!(result < u64::MAX - 4096, "native spawn failed: {path}");
        Ok(())
    }

    #[derive(Default)]
    pub struct Navigation {
        step: usize,
        initial: Option<(usize, usize)>,
    }
    impl Navigation {
        /// Drive one discrete action per settled frame.
        ///
        /// Each action's assertion runs on the *next* call, once the worker has
        /// settled, so the native gate proves the new button/dropdown/slider
        /// surfaces work in the real window rather than merely compiling.
        pub fn advance(
            &mut self,
            app: &mut sunlight_docview::ui::DocumentApp,
        ) -> anyhow::Result<bool> {
            if app.is_rendering() {
                return Ok(false);
            }
            anyhow::ensure!(
                app.cache_metrics().0 > 0 && app.cache_metrics().1 > 0,
                "native viewer has no rendered bitmap"
            );
            if self.initial.is_none() {
                self.initial = Some(app.cache_metrics());
                anyhow::ensure!(
                    app.displayed_page().1 == 6,
                    "native PDF fixture must have six pages"
                );
            }
            if self.step > 0 {
                self.verify(app)?;
            }
            let click = |rect: sunlight_ui::Rect| Event::Click {
                x: rect.x + (rect.w as i32 / 2),
                y: rect.y + (rect.h as i32 / 2),
            };
            let slider = app.slider_rect();
            if let Some((width, height)) = (match self.step {
                28 => Some((1000, 800)),
                29 => Some((800, 700)),
                _ => None,
            }) {
                self.step += 1;
                app.window_event(sunlight_ui::WindowEvent::Resized { width, height });
                return Ok(false);
            }
            let Some(event) = (match self.step {
                0..=4 => Some(Event::key_press(0x4d, true, false, false, false, false)),
                5..=9 => Some(Event::key_press(0x4b, true, false, false, false, false)),
                10 => Some(Event::Key('+')),
                11 => Some(Event::Key('-')),
                12 => Some(click(app.prev_rect())),
                13..=17 => Some(click(app.next_rect())),
                18 => Some(click(app.next_rect())),
                19 => Some(click(app.minus_rect())),
                20 => Some(click(app.plus_rect())),
                21 => Some(click(app.dropdown_rect())),
                22 => Some(Event::Click {
                    x: app.dropdown_item_rect(6).x + 4,
                    y: app.dropdown_item_rect(6).y + app.dropdown_item_rect(6).h as i32 / 2,
                }),
                23 => Some(Event::Click {
                    x: slider.x,
                    y: slider.y + slider.h as i32 / 2,
                }),
                24 => Some(Event::Click {
                    x: slider.right() - 1,
                    y: slider.y + slider.h as i32 / 2,
                }),
                25 => Some(click(app.prev_rect())),
                26 => Some(click(app.dropdown_rect())),
                27 => Some(Event::Click {
                    x: app.dropdown_item_rect(1).x + 4,
                    y: app.dropdown_item_rect(1).y + app.dropdown_item_rect(1).h as i32 / 2,
                }),
                _ => None,
            }) else {
                return Ok(true);
            };
            self.step += 1;
            app.update(event);
            Ok(false)
        }

        /// Assert the outcome of the action that produced the settled frame.
        fn verify(&self, app: &sunlight_docview::ui::DocumentApp) -> anyhow::Result<()> {
            let action = self.step - 1;
            let (page, count) = app.displayed_page();
            anyhow::ensure!(count == 6, "native viewer page count changed");
            let (mode, percent, _) = app.zoom_state();
            let fixed_in_range = matches!(mode, sunlight_docview::zoom::ZoomMode::Fixed(_))
                && (70..=200).contains(&percent);
            let action = action as u32;
            match action {
                // Keyboard forward then backward over every page.
                0..=4 => anyhow::ensure!(
                    page == action + 1,
                    "native keyboard navigation skipped a page"
                ),
                5..=9 => anyhow::ensure!(
                    page == 9 - action,
                    "native keyboard navigation skipped a page"
                ),
                10 | 11 => {}
                // Previous is disabled on the first page.
                12 => anyhow::ensure!(page == 0, "Previous was enabled on the first page"),
                // Next walks 2..=6, then is disabled on the last page.
                13..=17 => anyhow::ensure!(page == action - 12, "Next button skipped a page"),
                18 => anyhow::ensure!(page == 5, "Next was enabled on the last page"),
                19 | 20 => anyhow::ensure!(fixed_in_range, "zoom step left the slider range"),
                // Dropdown opens, then the 6th entry (125%) is chosen.
                21 => {}
                22 => anyhow::ensure!(percent == 125, "dropdown preset was not applied"),
                23 => anyhow::ensure!(
                    fixed_in_range && percent == 70,
                    "slider click did not select the minimum"
                ),
                24 => anyhow::ensure!(
                    fixed_in_range && percent == 200,
                    "slider click did not select the maximum"
                ),
                // Custom zoom must survive a page change.
                25 => anyhow::ensure!(
                    page == 4 && fixed_in_range && percent == 200,
                    "custom zoom did not persist across navigation"
                ),
                // Dropdown opens, then Fit Width is chosen.
                26 => {}
                27 => anyhow::ensure!(
                    matches!(mode, sunlight_docview::zoom::ZoomMode::FitWidth),
                    "Fit Width preset was not applied"
                ),
                // Window resize must re-resolve the fit zoom, in range.
                28 | 29 => anyhow::ensure!(
                    matches!(mode, sunlight_docview::zoom::ZoomMode::FitWidth)
                        && (70..=200).contains(&percent),
                    "resize did not keep a valid fit zoom"
                ),
                _ => {}
            }
            anyhow::ensure!(app.cache_metrics().0 <= 5, "native page cache is unbounded");
            anyhow::ensure!(
                app.cache_metrics().1 <= sunlight_docview::viewer::CACHE_BUDGET_BYTES,
                "native page cache exceeded its byte budget"
            );
            Ok(())
        }
    }
}
