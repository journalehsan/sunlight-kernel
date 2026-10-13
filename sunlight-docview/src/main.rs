use sunlight_ui::{
    app::{take_close_requested, App, Window, WindowConfig, WindowDecoration},
    Theme,
};

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
    use std::os::unix::ffi::OsStrExt;
    let mut path = None;
    for argument in arguments {
        if let Some(trace) = sunlight_ipc::launch_trace::parse_launch_arg(argument.as_bytes()) {
            sunlight_ipc::launch_trace::set_current(trace);
        } else if !argument.as_bytes().starts_with(b"--sunlight-launch=") && path.is_none() {
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
            inlateout("rax") sunlight_ipc::NATIVE_SYSCALL_TAG | 39 => result,
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
        pub fn advance(
            &mut self,
            app: &mut sunlight_docview::ui::DocumentApp,
        ) -> anyhow::Result<bool> {
            if app.is_rendering() {
                return Ok(false);
            }
            if self.initial.is_none() {
                self.initial = Some(app.cache_metrics());
            }
            anyhow::ensure!(app.cache_metrics().0 <= 5, "native page cache is unbounded");
            let event = match self.step {
                0..=4 => Event::key_press(0x4d, true, false, false, false, false),
                5..=9 => Event::key_press(0x4b, true, false, false, false, false),
                10 => Event::Key('+'),
                11 => Event::Key('-'),
                _ => {
                    anyhow::ensure!(
                        app.cache_metrics() == self.initial.context("initial cache missing")?,
                        "native cache grew between pages"
                    );
                    return Ok(true);
                }
            };
            self.step += 1;
            app.update(event);
            Ok(false)
        }
    }
    use anyhow::Context;
}
