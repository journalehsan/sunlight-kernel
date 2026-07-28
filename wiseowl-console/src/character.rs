use sunlight_ui::{Canvas, Color, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwlState {
    Idle,
    Listening,
    Thinking,
    Clarification,
    Confirmation,
    Acting,
    Observing,
    Success,
    Warning,
    Offline,
}

pub struct OwlCharacter {
    pub state: OwlState,
    pub tick: u64,
}

impl OwlCharacter {
    pub fn new() -> Self {
        Self {
            state: OwlState::Idle,
            tick: 0,
        }
    }

    pub fn update(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn draw(&self, canvas: &mut Canvas, rect: Rect) {
        // A minimal programmatic owl character to avoid heavy animation runtimes.
        let center_x = rect.x + rect.w as i32 / 2;
        let center_y = rect.y + rect.h as i32 / 2;
        
        let body_color = match self.state {
            OwlState::Offline => Color::rgb(100, 100, 100),
            OwlState::Warning => Color::rgb(200, 100, 100),
            OwlState::Success => Color::rgb(100, 200, 100),
            _ => Color::rgb(120, 100, 80),
        };

        // Draw body (simple rect as a placeholder for actual drawing primitives)
        canvas.fill_rect(Rect::new(center_x - 20, center_y - 20, 40, 40), body_color);
        
        // Draw eyes
        let eye_color = if self.state == OwlState::Offline {
            Color::rgb(50, 50, 50)
        } else {
            Color::rgb(255, 255, 200)
        };
        
        let mut left_eye_y = center_y - 10;
        let mut right_eye_y = center_y - 10;
        
        // Simple animation: blinking while idle, head tilt while listening
        match self.state {
            OwlState::Idle => {
                if self.tick % 100 < 5 { // Blink
                    left_eye_y = center_y - 5;
                    right_eye_y = center_y - 5;
                }
            }
            OwlState::Listening => {
                left_eye_y -= 2;
                right_eye_y += 2;
            }
            OwlState::Thinking => {
                let offset = (self.tick % 20) as i32 - 10;
                left_eye_y += offset / 5;
                right_eye_y += offset / 5;
            }
            _ => {}
        }
        
        canvas.fill_rect(Rect::new(center_x - 12, left_eye_y, 8, 8), eye_color);
        canvas.fill_rect(Rect::new(center_x + 4, right_eye_y, 8, 8), eye_color);
        
        // Draw beak
        canvas.fill_rect(
            Rect::new(center_x - 4, center_y + 2, 8, 6),
            Color::rgb(200, 150, 50),
        );
    }
}
