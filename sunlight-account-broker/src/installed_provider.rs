//! Trusted, opt-in interoperability fixture. Never populated from application IPC.
use crate::{Error, Provider, Result};

pub const REDIRECT_PORT: u16 = 53682;
pub const REDIRECT_URI: &str = "http://127.0.0.1:53682/oauth/callback";
pub const SCOPES: &[&str] = &["User.Read"];

/// Public registration identifier, compiled locally; no reusable client secret.
/// Missing or malformed registration fails closed before listener/browser launch.
pub fn microsoft() -> Result<Provider> {
    microsoft_with_client_id(option_env!("SUNLIGHT_MICROSOFT_CLIENT_ID").unwrap_or(""))
}
fn microsoft_with_client_id(client_id: &'static str) -> Result<Provider> {
    if client_id.len() != 36
        || !client_id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
        || client_id == "00000000-0000-0000-0000-000000000000"
    {
        return Err(Error::Configuration);
    }
    Ok(Provider {
        id: "microsoft-consumers",
        authorization_endpoint: "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize",
        token_endpoint: "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
        client_id,
        allowed_scopes: SCOPES,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_is_bounded_and_endpoints_are_fixed() {
        for id in [
            "",
            "https://attacker.invalid",
            "00000000-0000-0000-0000-000000000000",
            "11111111-1111-1111-1111-11111111111g",
        ] {
            assert!(matches!(
                microsoft_with_client_id(id),
                Err(Error::Configuration)
            ));
        }
        let p = microsoft_with_client_id("11111111-1111-1111-1111-111111111111").unwrap();
        assert_eq!(p.allowed_scopes, &["User.Read"]);
        assert_eq!(
            p.token_endpoint,
            "https://login.microsoftonline.com/consumers/oauth2/v2.0/token"
        );
        assert_eq!(
            REDIRECT_URI,
            alloc::format!("http://127.0.0.1:{REDIRECT_PORT}/oauth/callback")
        );
    }
}
