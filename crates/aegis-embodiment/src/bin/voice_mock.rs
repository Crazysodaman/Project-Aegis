//! Trivial test-only executable used by the voice command-adapter tests.
//!
//! It never performs audio I/O. It proves that the adapter passes fixed
//! arguments as a vector, pipes stdin to stdout, and enforces timeouts and
//! output caps. This binary is not a TTS or STT engine.

use std::io::{Read, Write};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("echo") => echo(),
        Some("sleep") => sleep(args.get(1)),
        Some("fail") => fail(),
        Some("big") => big(args.get(1)),
        _ => std::process::exit(2),
    }
}

fn echo() {
    let mut data = Vec::new();
    if std::io::stdin().read_to_end(&mut data).is_err() {
        std::process::exit(1);
    }
    if std::io::stdout().write_all(&data).is_err() {
        std::process::exit(1);
    }
}

fn sleep(milliseconds: Option<&String>) {
    let milliseconds = milliseconds
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(10_000);
    std::thread::sleep(Duration::from_millis(milliseconds));
}

fn fail() {
    eprintln!("voice mock failure");
    std::process::exit(3);
}

fn big(bytes: Option<&String>) {
    let bytes = bytes
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1024);
    let chunk = vec![b'x'; 8192];
    let mut stdout = std::io::stdout();
    let mut remaining = bytes;
    while remaining > 0 {
        let take = remaining.min(chunk.len());
        if stdout.write_all(&chunk[..take]).is_err() {
            break;
        }
        remaining -= take;
    }
}
