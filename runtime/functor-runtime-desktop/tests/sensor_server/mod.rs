//! An in-test HTTP server for the sensor stream tests (`depth_stream`,
//! `camera_stream`).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// An HTTP server that records each POST body of `content_type`, answering
/// 204 after `delay` — or never, when `silent`.
pub struct Server {
    pub url: String,
    bodies: Arc<Mutex<Vec<Vec<u8>>>>,
    pub max_in_flight: Arc<AtomicUsize>,
}

impl Server {
    pub fn start(content_type: &'static str, delay: Duration, silent: bool) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/frames", listener.local_addr().unwrap());
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let max_in_flight = Arc::new(AtomicUsize::new(0));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let (thread_bodies, thread_max) = (bodies.clone(), max_in_flight.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let (bodies, max, in_flight) =
                    (thread_bodies.clone(), thread_max.clone(), in_flight.clone());
                std::thread::spawn(move || {
                    let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    max.fetch_max(now, Ordering::SeqCst);
                    let body = read_request(&mut stream, content_type);
                    if silent {
                        std::thread::sleep(Duration::from_secs(5));
                    } else {
                        std::thread::sleep(delay);
                        bodies.lock().unwrap().extend(body);
                        let _ = stream
                            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");
                    }
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        Server {
            url,
            bodies,
            max_in_flight,
        }
    }

    pub fn bodies(&self) -> Vec<Vec<u8>> {
        self.bodies.lock().unwrap().clone()
    }
}

/// A request's body, by its `Content-Length`.
fn read_request(stream: &mut std::net::TcpStream, content_type: &str) -> Option<Vec<u8>> {
    let mut data = Vec::new();
    let mut chunk = [0u8; 65536];
    let head_end = loop {
        if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        let n = stream.read(&mut chunk).ok().filter(|n| *n > 0)?;
        data.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&data[..head_end]).to_ascii_lowercase();
    assert!(head.starts_with("post /frames http/1.1"), "{head}");
    assert!(
        head.contains(&format!("content-type: {content_type}")),
        "{head}"
    );
    let len: usize = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())?;
    while data.len() < head_end + len {
        let n = stream.read(&mut chunk).ok().filter(|n| *n > 0)?;
        data.extend_from_slice(&chunk[..n]);
    }
    Some(data[head_end..head_end + len].to_vec())
}
