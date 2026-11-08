#![cfg(feature = "host-linux")]
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
};
use sunlight_fetch::{backend::perform_request_filtered, FetchError};
use sunlight_http::{HttpRequest, ParsedUrl};
fn request(url: &ParsedUrl) -> HttpRequest {
    HttpRequest {
        method: "GET",
        path: url.path.clone(),
        host: url.host_header(),
        headers: Vec::new(),
        body: None,
    }
}
#[test]
fn blocked_initial_request_never_connects() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = ParsedUrl::parse(&format!(
        "http://{}/blocked",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let result = perform_request_filtered(url.clone(), request(&url), 4096, &mut |_| {
        Err(FetchError::BlockedByPolicy)
    });
    assert!(matches!(result, Err(FetchError::BlockedByPolicy)));
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
#[test]
fn redirected_block_never_reaches_destination() {
    let destination = TcpListener::bind("127.0.0.1:0").unwrap();
    destination.set_nonblocking(true).unwrap();
    let initial = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = initial.local_addr().unwrap();
    let target = destination.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut s, _) = initial.accept().unwrap();
        let mut bytes = [0; 4096];
        s.read(&mut bytes).unwrap();
        write!(s,"HTTP/1.1 302 Found\r\nLocation: http://{target}/ads/banner\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    let seen = Arc::new(AtomicUsize::new(0));
    let url = ParsedUrl::parse(&format!("http://{address}/good")).unwrap();
    let result = perform_request_filtered(url.clone(), request(&url), 4096, &mut |url| {
        seen.fetch_add(1, Ordering::Relaxed);
        if url.port == target.port() {
            Err(FetchError::BlockedByPolicy)
        } else {
            Ok(())
        }
    });
    server.join().unwrap();
    assert_eq!(seen.load(Ordering::Relaxed), 2);
    assert!(matches!(result, Err(FetchError::BlockedByPolicy)));
    assert_eq!(
        destination.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
