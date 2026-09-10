//! A stand-in for `ckb-light-client`, used only by supervisor tests.
//!
//! Reads the generated config for its RPC port and answers `local_node_info`.
//! Env vars steer the failure modes the supervisor must handle:
//!   `FAKE_LC_NEVER_READY=1` — bind nothing, so readiness times out
//!   `FAKE_LC_EXIT_AFTER_MS=n` — serve, then exit(1) to force a restart

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config_path = args
        .iter()
        .position(|a| a == "--config-file")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let config = std::fs::read_to_string(config_path.expect("--config-file is required"))
        .expect("config file is readable");
    let addr = config
        .lines()
        .find_map(|l| l.trim().strip_prefix("listen_address = "))
        .map(|v| v.trim().trim_matches('"').to_string())
        .expect("config carries an rpc listen_address");

    eprintln!("fake light client starting on {addr}");

    if std::env::var("FAKE_LC_NEVER_READY").is_ok() {
        std::thread::sleep(Duration::from_secs(3600));
        return;
    }

    let listener = TcpListener::bind(&addr).expect("bind the configured port");
    println!("fake light client ready");

    let deadline = std::env::var("FAKE_LC_EXIT_AFTER_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|ms| Instant::now() + Duration::from_millis(ms));
    listener
        .set_nonblocking(true)
        .expect("non-blocking so the deadline can be honoured");

    loop {
        if let Some(deadline) = deadline
            && Instant::now() >= deadline
        {
            eprintln!("fake light client exiting on purpose");
            std::process::exit(1);
        }
        match listener.accept() {
            Ok((mut sock, _)) => {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf);
                let body =
                    br#"{"jsonrpc":"2.0","id":1,"result":{"version":"fake","node_id":"fake"}}"#;
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(head.as_bytes());
                let _ = sock.write_all(body);
                let _ = sock.flush();
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
}
