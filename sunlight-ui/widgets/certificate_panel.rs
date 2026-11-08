//! Certificate details supplied by the network backend.
use crate::{geom::Rect, paint::Canvas, theme::Theme};
use alloc::{format, string::String, vec::Vec};
use sunlight_cert::{CertInfo, ConnectionSecurity};

#[derive(Default)]
pub struct CertificatePanel {
    pub open: bool,
    pub expanded: bool,
    pub scroll: usize,
}
impl CertificatePanel {
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
        let columns = (width.saturating_sub(24) as usize / 8).max(1);
        let mut lines = Vec::new();
        for row in self.rows(security, cert, error) {
            let mut line = String::new();
            let mut count = 0;
            for ch in row.chars() {
                if count == columns {
                    lines.push(core::mem::take(&mut line));
                    count = 0;
                }
                line.push(if ch.is_control() { ' ' } else { ch });
                count += 1;
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
        let max_rows = rect.h.saturating_sub(52) / 20;
        for (index, row) in self
            .lines(rect.w, security, cert, error)
            .iter()
            .skip(self.scroll)
            .take(max_rows as usize)
            .enumerate()
        {
            let text = row;
            canvas.draw_text(
                rect.x + 12,
                rect.y + 12 + index as i32 * 20,
                text,
                if index + self.scroll == 0 && security == ConnectionSecurity::Invalid {
                    theme.warn
                } else {
                    theme.text
                },
            );
        }
        let button = Self::details_rect(rect);
        canvas.fill_rounded_rect(button, 4, theme.panel_alt);
        canvas.draw_text(
            button.x + 6,
            button.y + 7,
            if self.expanded {
                "Hide Details"
            } else {
                "View Full Chain"
            },
            theme.accent,
        );
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
