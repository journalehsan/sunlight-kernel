//! Certificate details supplied by the network backend.
use crate::{font::VecText, geom::Rect, paint::Canvas, theme::Theme};
use alloc::{format, string::String, vec::Vec};
use sunlight_cert::{CertInfo, ConnectionSecurity};

#[derive(Default)]
pub struct CertificatePanel<'a> {
    pub open: bool,
    pub expanded: bool,
    pub scroll: usize,
    font: Option<&'a dyn VecText>,
}
impl<'a> CertificatePanel<'a> {
    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        self
    }

    fn measure_text(&self, text: &str) -> u32 {
        self.font
            .map_or_else(|| Canvas::measure_text(text), |font| font.measure_w(text))
    }
    pub fn rows(
        &self,
        security: ConnectionSecurity,
        cert: Option<&CertInfo>,
        error: Option<&str>,
    ) -> Vec<String> {
        let mut rows = alloc::vec![String::from(match security {
            ConnectionSecurity::Secure => "Secure connection",
            ConnectionSecurity::Invalid => "Warning: Invalid certificate",
            ConnectionSecurity::NotSecure => "Not Secure",
            ConnectionSecurity::Unknown => "Connection details unavailable",
        })];
        if let Some(error) = error {
            rows.push(String::from(error));
        }
        if let Some(cert) = cert {
            if let Some(host) = &cert.verified_host {
                rows.push(format!("Domain: {host}"));
            }
            rows.push(format!("Subject: {}", cert.subject));
            rows.push(format!("Issuer: {}", cert.issuer));
            rows.push(format!(
                "From: {}",
                cert.valid_from.as_deref().unwrap_or("Unavailable")
            ));
            rows.push(format!(
                "Until: {}",
                cert.valid_to.as_deref().unwrap_or("Unavailable")
            ));
            rows.push(format!(
                "Signature: {}",
                cert.signature_algorithm.as_deref().unwrap_or("Unavailable")
            ));
            rows.push(format!(
                "Public key: {}",
                cert.key_bits
                    .map(|n| format!("{n} bits"))
                    .unwrap_or(String::from("Unavailable"))
            ));
            rows.extend(cert.validation_errors.iter().cloned());
            if self.expanded {
                rows.push(String::from("Available chain (root-most first):"));
                if !cert
                    .chain
                    .iter()
                    .any(|c| c.trust == sunlight_cert::CertificateTrust::TrustAnchor)
                {
                    rows.push(String::from("Root certificate details unavailable"));
                }
                for entry in &cert.chain {
                    rows.push(format!(
                        "{} {}",
                        match entry.trust {
                            sunlight_cert::CertificateTrust::TrustAnchor => "[Trusted anchor]",
                            sunlight_cert::CertificateTrust::SessionSupplied => "[TLS supplied]",
                            sunlight_cert::CertificateTrust::Unverified => "[Unverified]",
                        },
                        entry.subject
                    ));
                }
            }
        } else if security == ConnectionSecurity::Secure {
            rows.push(String::from("Certificate metadata unavailable"));
        }
        rows
    }
    pub fn lines(
        &self,
        width: u32,
        security: ConnectionSecurity,
        cert: Option<&CertInfo>,
        error: Option<&str>,
    ) -> Vec<String> {
        let available_width = width.saturating_sub(24);
        let mut lines = Vec::new();
        for row in self.rows(security, cert, error) {
            let mut line = String::new();
            for ch in row.chars() {
                let ch = if ch.is_control() { ' ' } else { ch };
                line.push(ch);
                if self.measure_text(&line) > available_width && line.len() > ch.len_utf8() {
                    line.pop();
                    lines.push(core::mem::take(&mut line));
                    line.push(ch);
                }
            }
            lines.push(line);
        }
        lines
    }

    pub fn details_rect(rect: Rect) -> Rect {
        Rect::new(
            rect.x + 12,
            rect.bottom() - 32,
            rect.w.saturating_sub(24),
            24,
        )
    }
    pub fn draw(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        rect: Rect,
        security: ConnectionSecurity,
        cert: Option<&CertInfo>,
        error: Option<&str>,
    ) {
        if !self.open {
            return;
        }
        canvas.fill_rounded_rect(rect, 8, theme.panel);
        canvas.stroke_rounded_rect(rect, 8, 1, theme.border);
        let row_height = self.font.map_or(20, |font| font.line_height().max(20));
        let max_rows = rect.h.saturating_sub(52) / row_height;
        for (index, row) in self
            .lines(rect.w, security, cert, error)
            .iter()
            .skip(self.scroll)
            .take(max_rows as usize)
            .enumerate()
        {
            let color = if index + self.scroll == 0 && security == ConnectionSecurity::Invalid {
                theme.warn
            } else {
                theme.text
            };
            let y = rect.y + 12 + index as i32 * row_height as i32;
            if let Some(font) = self.font {
                font.draw_vcenter(canvas, row, rect.x + 12, y, row_height, color);
            } else {
                canvas.draw_text(rect.x + 12, y, row, color);
            }
        }
        let button = Self::details_rect(rect);
        canvas.fill_rounded_rect(button, 4, theme.panel_alt);
        let label = if self.expanded {
            "Hide Details"
        } else {
            "View Full Chain"
        };
        if let Some(font) = self.font {
            font.draw_vcenter(
                canvas,
                label,
                button.x + 6,
                button.y,
                button.h,
                theme.accent,
            );
        } else {
            canvas.draw_text(button.x + 6, button.y + 7, label, theme.accent);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renders_backend_fields_and_security_states() {
        let mut panel = CertificatePanel::default();
        let cert = sunlight_cert::parse_der(
            include_bytes!("../../sunlight-tls/src/roots/gts_root_r4.der"),
            true,
        )
        .unwrap();
        let rows = panel.rows(ConnectionSecurity::Secure, Some(&cert), None);
        assert!(rows.iter().any(|r| r.contains("GTS Root R4")));
        assert!(rows.iter().any(|r| r.contains("384 bits")));
        assert_eq!(
            panel.rows(ConnectionSecurity::NotSecure, None, None)[0],
            "Not Secure"
        );
        assert!(
            panel.rows(ConnectionSecurity::Invalid, None, Some("Expired"))[1].contains("Expired")
        );
        panel.expanded = true;
        assert!(panel
            .rows(ConnectionSecurity::Secure, Some(&cert), None)
            .iter()
            .any(|r| r.contains("Available chain")));
        panel.open = true;
        let mut pixels = alloc::vec![0u32; 560 * 480];
        let mut canvas = Canvas::new(&mut pixels, 560, 560, 480);
        panel.draw(
            &mut canvas,
            &Theme::sunlight_dark(),
            Rect::new(0, 0, 560, 480),
            ConnectionSecurity::Secure,
            Some(&cert),
            None,
        );
        assert!(pixels.iter().any(|p| *p != 0));
    }
}
