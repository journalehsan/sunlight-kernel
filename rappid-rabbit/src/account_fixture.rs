//! Compiled-in local application; never selected by URL or downloaded content.
use crate::{
    css::{collect_embedded_stylesheets, StyleContext},
    dom_fixture::{FixtureKey, FixtureSession, KeyModifiers},
    render::{DocumentRenderState, TextMeasurer},
};
use sun_font::{FontRole, VecFont};
use sunlight_ui::font::VecText;
static FIXTURE_FONT: VecFont = VecFont(FontRole::UiRegular);
use sunlight_ui::{
    app::{App, Window, WindowConfig, WindowDecoration},
    widgets::{DocumentCanvas, DocumentCanvasPresentation},
    Canvas, Event, Point, Rect, Size, Theme,
};

pub const HTML: &str = include_str!("../fixtures/account.html");
pub const SCRIPT: &str = include_str!("../fixtures/account.js");
pub struct Measure;
impl TextMeasurer for Measure {
    fn measure_width(&self, text: &str) -> u32 {
        FIXTURE_FONT.measure_w(text)
    }
    fn line_height(&self) -> u32 {
        VecText::line_height(&FIXTURE_FONT)
    }
}
pub fn document(generation: u64) -> DocumentRenderState {
    let dom = golden_fish::parse_html(HTML).expect("compiled fixture HTML");
    let styles = StyleContext::build_with_viewport(&dom, &collect_embedded_stylesheets(&dom), 640);
    DocumentRenderState::new(
        generation,
        "fixture:account".into(),
        dom,
        styles,
        Size::new(640, 700),
        &Measure,
    )
}
/// UI-owned session is deliberately separate from Rabbit's Send navigation worker.
pub struct AccountFixture {
    pub render: DocumentRenderState,
    pub session: FixtureSession,
    failed: bool,
    modifiers: KeyModifiers,
}
impl AccountFixture {
    pub fn new() -> sunlight_js::JsResult<Self> {
        let mut render = document(1);
        let mut session = FixtureSession::new(&render)?;
        session.evaluate(&mut render, SCRIPT, &Measure)?;
        Ok(Self {
            render,
            session,
            failed: false,
            modifiers: KeyModifiers::default(),
        })
    }
    fn canvas(&self) -> DocumentCanvas<'_> {
        DocumentCanvas::new(
            Rect::new(0, 0, self.render.viewport.w, self.render.viewport.h),
            &[],
        )
        .with_presentation(DocumentCanvasPresentation::Browser)
        .with_fonts(None, None, Some(&FIXTURE_FONT), Some(&FIXTURE_FONT))
        .with_scene(&self.render.current_scene)
    }
}
impl App for AccountFixture {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        self.canvas().draw(canvas, theme);
    }
    fn update(&mut self, event: Event) -> bool {
        if self.failed {
            return false;
        }
        if let Event::KeyPress {
            shift,
            ctrl,
            alt,
            super_key,
            ..
        } = event
        {
            self.modifiers = KeyModifiers {
                shift,
                ctrl,
                alt,
                meta: super_key,
            };
        }
        if let Event::FocusChanged { focused: false } = event {
            self.modifiers = KeyModifiers::default();
        }
        let result = match event {
            Event::Tick => {
                return match self.session.complete(&mut self.render, true, &Measure) {
                    Ok(changed) => changed,
                    Err(_) => {
                        self.failed = true;
                        true
                    }
                }
            }
            Event::Click { x, y } => {
                let node = self
                    .canvas()
                    .hit_test(Point::new(x, y))
                    .map(|o| o.owner_node_id.0 as usize);
                if let Some(node) = node {
                    self.session.click(&mut self.render, node, &Measure)
                } else {
                    return false;
                }
            }
            Event::Key(c) => {
                let key = match c {
                    '\n' | '\r' => FixtureKey::Enter,
                    '\u{8}' => FixtureKey::Backspace,
                    '\t' => FixtureKey::Tab(false),
                    ' ' => FixtureKey::Space,
                    c if !c.is_control() => FixtureKey::Text(c),
                    _ => return false,
                };
                self.session
                    .key_with_modifiers(&mut self.render, key, self.modifiers, &Measure)
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                shift,
                ..
            } => {
                // Decoded Event::Key owns text/Enter/Backspace; raw transitions own navigation.
                let key = match keycode {
                    0x1c => FixtureKey::Enter,
                    0x01 => FixtureKey::Escape,
                    0x48 => FixtureKey::Up,
                    0x50 => FixtureKey::Down,
                    0x4b => FixtureKey::Left,
                    0x4d => FixtureKey::Right,
                    0x47 => FixtureKey::Home,
                    0x4f => FixtureKey::End,
                    0x53 => FixtureKey::Delete,
                    0x0f => FixtureKey::Tab(shift),
                    _ => return false,
                };
                self.session
                    .key_with_modifiers(&mut self.render, key, self.modifiers, &Measure)
            }
            _ => return false,
        };
        // Never format JS errors: source text or thrown values can contain input.
        if result.is_err() {
            self.failed = true;
        }
        true
    }
}
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = AccountFixture::new().map_err(|_| "fixture initialization failed")?;
    let mut window = Window::connect(WindowConfig {
        width: 640,
        height: 700,
        title: "Rappid Rabbit — Account Lab",
        decoration: WindowDecoration::Normal,
    })
    .ok_or("failed to connect window")?;
    window.run(&mut app);
    Ok(())
}
