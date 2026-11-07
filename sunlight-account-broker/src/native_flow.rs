//! Native listener/session coordinator for opt-in mock/provider test images.
//! Shell test flows stay transient. Panel flows commit to UID-scoped live KV.
use crate::{native_net::Socket, NativeEntropy};
use alloc::{format, string::String, vec::Vec};
use sunlight_account_broker::{
    storage, transport::VerifiedTls, Broker, DenyAccess, Error, KvClient, MemoryKv, Provider,
    Started,
};
use sunlight_ipc::{debug_log, monotonic_millis};

type Core = Broker<MemoryKv, NativeEntropy, VerifiedTls, DenyAccess>;
pub struct Flow {
    pub id: u64,
    pub owner: u64,
    pub manager: Option<sunlight_ipc::AccountBrokerCaller>,
    pub browser: u64,
    mode: u64,
    broker: Core,
    started: Started,
    listener: Option<Socket>,
    redirect: String,
    pub url: Option<String>,
    pub url_page: Option<sunlight_ipc::CapabilityToken>,
    pub status: u64,
    deadline: u64,
}
impl Flow {
    #[cfg(feature = "native-mock")]
    pub fn start(id: u64, owner: u64, port: u16, mode: u64) -> Result<Self, Error> {
        let case = match mode {
            0 => "success",
            11 => "interactive",
            1 => "wrong-state",
            2 => "replay",
            3 => "timeout",
            4 => "cancel",
            5 => "denied",
            6 => "token-error",
            7 => "token-timeout",
            8 => "network-error",
            9 => "exchange-timeout",
            _ => return Err(Error::Configuration),
        };
        if port != 8443 {
            return Err(Error::Configuration);
        }
        let authorization_endpoint = match case {
            "success" => "https://10.0.2.2:8443/authorize/success",
            "interactive" => "https://10.0.2.2:8443/authorize/interactive",
            "wrong-state" => "https://10.0.2.2:8443/authorize/wrong-state",
            "replay" => "https://10.0.2.2:8443/authorize/replay",
            "timeout" => "https://10.0.2.2:8443/authorize/timeout",
            "cancel" => "https://10.0.2.2:8443/authorize/cancel",
            "denied" => "https://10.0.2.2:8443/authorize/denied",
            "token-error" => "https://10.0.2.2:8443/authorize/token-error",
            "token-timeout" => "https://10.0.2.2:8443/authorize/token-timeout",
            "network-error" => "https://10.0.2.2:8443/authorize/network-error",
            _ => "https://10.0.2.2:8443/authorize/exchange-timeout",
        };
        let token_endpoint = if mode == 8 {
            "https://10.0.2.2:1/token"
        } else {
            "https://10.0.2.2:8443/token"
        };
        Self::launch(
            id,
            owner,
            mode,
            0,
            Provider {
                id: "native-mock",
                authorization_endpoint,
                token_endpoint,
                client_id: "sunlight-native-mock",
                allowed_scopes: &["mail.read"],
            },
            &["mail.read"],
        )
    }
    #[cfg(feature = "native-provider-test")]
    pub fn start_provider(id: u64, owner: u64) -> Result<Self, Error> {
        use sunlight_account_broker::installed_provider as installed;
        let provider = installed::microsoft().map_err(|e| {
            debug_log("[ACCOUNT-BROKER] real provider blocked: local Microsoft client ID missing or invalid");
            e
        })?;
        Self::launch(
            id,
            owner,
            10,
            installed::REDIRECT_PORT,
            provider,
            installed::SCOPES,
        )
    }
    fn launch(
        id: u64,
        owner: u64,
        mode: u64,
        port: u16,
        provider: Provider,
        scopes: &[&str],
    ) -> Result<Self, Error> {
        let now = monotonic_millis();
        let (listener, local) = Socket::bind_loopback_port(port, now + 2000)?;
        let redirect = format!("http://127.0.0.1:{local}/oauth/callback");
        if mode == 10 && redirect != sunlight_account_broker::installed_provider::REDIRECT_URI {
            return Err(Error::Configuration);
        }
        let mut broker = Broker::new(
            provider,
            MemoryKv::default(),
            NativeEntropy,
            VerifiedTls,
            DenyAccess,
        )?;
        let started = broker.start_authorization(scopes, &redirect, now / 1000)?;
        let url = Some(started.authorization_url.clone());
        // Only a short marker and opaque flow number enter argv. URL transfer
        // uses broker IPC, authenticated against this exact spawned PID.
        let arg = format!("{id}");
        let browser = sunlight_libc::spawn(
            b"/bin/rappid-rabbit",
            &[b"rappid-rabbit", b"--account-broker", arg.as_bytes()],
            None,
        )
        .map_err(|_| Error::Browser)?;
        debug_log("[ACCOUNT-BROKER] listener ready before native browser launch");
        Ok(Self {
            id,
            owner,
            manager: None,
            browser,
            mode,
            broker,
            started,
            listener: Some(listener),
            redirect,
            url,
            url_page: None,
            status: 0,
            deadline: now
                + if mode == 3 || mode == 4 {
                    4000
                } else {
                    120_000
                },
        })
    }
    pub fn release_url_page(&mut self) {
        if let Some(token) = self.url_page.take() {
            let _ = sunlight_ipc::shm_free(token);
        }
    }
    pub fn close_browser(&mut self) {
        self.release_url_page();
        if self.browser != 0 {
            let _ = sunlight_libc::kill(self.browser, 15);
            self.browser = 0;
        }
    }
    pub fn cancel(&mut self) {
        if self.status != 0 {
            return;
        }
        let _ = self.broker.cancel_authorization(&self.started.session);
        self.listener = None;
        self.url = None;
        self.status = 4;
        self.close_browser();
        debug_log("[ACCOUNT-BROKER] cancelled; listener and session released");
    }
    pub fn tick(&mut self) {
        if self.status != 0 {
            self.close_browser();
            return;
        }
        let now = monotonic_millis();
        if now >= self.deadline {
            self.cancel();
            self.status = 3;
            debug_log("[ACCOUNT-BROKER] callback timeout; listener and session released");
            return;
        }
        let result = (|| {
            let Some(listener) = self.listener.as_ref() else {
                return Err(Error::Callback);
            };
            let Some(socket) = listener.accept(now + 500)? else {
                return Ok(None);
            };
            if socket.endpoint(true, now + 500)?.0 != [127, 0, 0, 1] {
                return Err(Error::Callback);
            }
            let raw = socket.read_request((now + 2000).min(self.deadline))?;
            let callback = callback_url(&raw, &self.redirect)?;
            if self.mode == 10 {
                debug_log("[ACCOUNT-BROKER] real provider loopback callback received; verified HTTPS exchange pending");
            }
            let result = self.broker.complete_session(
                &self.started.session,
                &callback,
                monotonic_millis() / 1000,
            );
            // Fetch consumes this native terminal redirect before events/render
            // and supplies a static credential-free page. No callback data in JS.
            let _=socket.send(b"HTTP/1.1 303 See Other\r\nLocation: about:blank\r\nContent-Length: 0\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",monotonic_millis()+1000);
            match result {
                Ok(account) => {
                    if let Some(manager) = self.manager {
                        // Only the broker sees this token response. Persist a validated,
                        // UID-scoped record, then the index, before reporting completion.
                        let raw = self.broker.kv.get(&account.account)?;
                        let mut token: serde_json::Value =
                            serde_json::from_str(&raw).map_err(|_| Error::MalformedToken)?;
                        let result = (|| {
                            let identifier = if self.mode == 10 {
                                "Microsoft account"
                            } else {
                                "mock@sunlight.test"
                            };
                            let scopes: Vec<String> = if self.mode == 10 {
                                sunlight_account_broker::installed_provider::SCOPES
                            } else {
                                &["mail.read"]
                            }
                            .iter()
                            .map(|s| String::from(*s))
                            .collect();
                            let record = storage::encode_record(
                                if self.mode == 10 {
                                    "microsoft-consumers"
                                } else {
                                    "native-mock"
                                },
                                identifier,
                                &scopes,
                                monotonic_millis() / 1000
                                    + token["expires_in"].as_u64().ok_or(Error::MalformedToken)?,
                                token["access_token"]
                                    .as_str()
                                    .ok_or(Error::MalformedToken)?,
                                token["token_type"].as_str().ok_or(Error::MalformedToken)?,
                                token["refresh_token"].as_str(),
                            )?;
                            let mut kv = crate::native_store::LiveKv(manager.uid);
                            kv.put(&storage::record_key(&account.account), &record)?;
                            if let Err(e) = storage::index_add(&mut kv, &account.account) {
                                let _ = kv.delete(&storage::record_key(&account.account));
                                return Err(e);
                            }
                            Ok(())
                        })();
                        use zeroize::Zeroize;
                        for key in ["access_token", "refresh_token"] {
                            if let Some(serde_json::Value::String(s)) = token.get_mut(key) {
                                s.zeroize();
                            }
                        }
                        result?;
                        self.broker.disconnect(&account.account)?;
                        debug_log("[ONLINE-ACCOUNTS] session record committed; metadata ready");
                        return Ok(Some(1));
                    }
                    if self.mode == 10 {
                        self.broker.disconnect(&account.account)?;
                        debug_log("[ACCOUNT-BROKER] real provider verified HTTPS code exchange complete; transient tokens discarded; access denied");
                        return Ok(Some(1));
                    }
                    if self.broker.complete_session(
                        &self.started.session,
                        &callback,
                        monotonic_millis() / 1000,
                    ) != Err(Error::State)
                    {
                        return Err(Error::State);
                    }
                    if self.mode == 2 {
                        let until = monotonic_millis() + 3000;
                        let port = self
                            .redirect
                            .strip_prefix("http://127.0.0.1:")
                            .unwrap()
                            .split('/')
                            .next()
                            .unwrap()
                            .parse()
                            .map_err(|_| Error::Callback)?;
                        let client = Socket::new(until)?;
                        client.connect_loopback(port, until)?;
                        let replay = loop {
                            if let Some(peer) = listener.accept(until)? {
                                break peer;
                            }
                            sunlight_ipc::process_yield();
                        };
                        client.send(raw.as_bytes(), until)?;
                        let duplicate = replay.read_request(until)?;
                        let duplicate = callback_url(&duplicate, &self.redirect)?;
                        if self.broker.complete_session(
                            &self.started.session,
                            &duplicate,
                            monotonic_millis() / 1000,
                        ) != Err(Error::State)
                            || self.broker.kv.writes != 1
                        {
                            return Err(Error::State);
                        }
                        debug_log("[ACCOUNT-BROKER] native HTTP callback replay rejected; volatile writes=1");
                    }
                    debug_log("[ACCOUNT-BROKER] replay rejected by consumed native session");
                    debug_log("[ACCOUNT-BROKER] native mock code exchanged; completion; volatile writes=1");
                    Ok(Some(1))
                }
                Err(error) => Err(error),
            }
        })();
        match result {
            Ok(None) => {}
            Ok(Some(status)) => {
                self.status = status;
                self.listener = None;
                self.url = None;
            }
            Err(error) => {
                let _ = self.broker.cancel_authorization(&self.started.session);
                self.listener = None;
                self.url = None;
                self.status = match error {
                    Error::State => 2,
                    Error::Timeout | Error::Expired => 3,
                    Error::Provider => 5,
                    Error::Kv | Error::Capacity | Error::Unauthorized => 7,
                    _ => 6,
                };
                debug_log(match error {
                    Error::State => "[ACCOUNT-BROKER] callback rejected: state",
                    Error::Timeout | Error::Expired => "[ACCOUNT-BROKER] native timeout",
                    Error::Provider => "[ACCOUNT-BROKER] provider failure",
                    _ => "[ACCOUNT-BROKER] native network/callback failure",
                });
            }
        }
    }
}

fn callback_url(raw: &str, redirect: &str) -> Result<String, Error> {
    if raw.len() > 8192 || !raw.ends_with("\r\n\r\n") {
        return Err(Error::Callback);
    }
    let mut lines = raw.split("\r\n");
    let mut request = lines.next().ok_or(Error::Callback)?.split(' ');
    if request.next() != Some("GET") {
        return Err(Error::Callback);
    }
    let path = request.next().ok_or(Error::Callback)?;
    if request.next() != Some("HTTP/1.1")
        || request.next().is_some()
        || !path.starts_with("/oauth/callback?")
    {
        return Err(Error::Callback);
    }
    let hosts: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(k, _)| k.eq_ignore_ascii_case("host"))
        .collect();
    let authority = redirect
        .strip_prefix("http://")
        .unwrap()
        .split('/')
        .next()
        .unwrap();
    if hosts.len() != 1 || hosts[0].1.trim() != authority {
        return Err(Error::Callback);
    }
    Ok(format!("http://{authority}{path}"))
}

impl Drop for Flow {
    fn drop(&mut self) {
        let _ = self.broker.cancel_authorization(&self.started.session);
        self.close_browser();
    }
}
