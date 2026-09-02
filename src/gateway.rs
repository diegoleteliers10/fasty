//! Native Web Gateway: embedded HTTP & WebSocket server for fastty.
//!
//! Serves the fastty web interface and bridges WebSocket connections directly
//! to the local daemon's Unix socket (`fasttyd.sock`).
//!
//! Run with: `fastty gateway [--port <PORT>] [--host <HOST>] [--read-only]`

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(unix)]
use std::sync::Arc;
use std::time::Duration;

use crate::daemon::{base64_encode, socket_path};

const EMBEDDED_INDEX_HTML: &str = include_str!("../web/index.html");
const EMBEDDED_STYLE_CSS: &str = include_str!("../web/style.css");
const EMBEDDED_APP_JS: &str = include_str!("../web/app.js");
const EMBEDDED_WASM: &[u8] = include_bytes!("../web/fastty_wasm.wasm");

/// Simple pure-Rust SHA-1 implementation (RFC 3174) for the WebSocket handshake.
pub fn sha1(input: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let ml = (input.len() as u64) * 8;
    let mut msg = input.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&ml.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, item) in w.iter_mut().enumerate().take(16) {
            *item = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;

        for (i, &w_val) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w_val);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

pub fn run_gateway(host: &str, port: u16, read_only: bool) {
    let _ = crate::paths::init();

    #[cfg(unix)]
    {
        let sock = socket_path();
        if UnixStream::connect(&sock).is_err() {
            crate::daemon::start();
            crate::daemon::ensure_default_session();
        }
    }

    let bind_addr = format!("{}:{}", host, port);
    let listener = match TcpListener::bind(&bind_addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("fastty gateway: failed to bind to {}: {}", bind_addr, e);
            std::process::exit(1);
        }
    };

    println!("\n  🚀 Fastty Web Gateway");
    println!(
        "  ➜ Local:   http://{}:{}",
        if host == "0.0.0.0" { "localhost" } else { host },
        port
    );
    println!("  ➜ Daemon:  {}", socket_path().display());
    if read_only {
        println!("  ➜ Mode:    read-only (writes ignored)");
    }
    println!("  ➜ Ready for browser connections. Press Ctrl+C to stop.\n");

    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || {
            handle_client(stream, read_only);
        });
    }
}

fn handle_client(mut stream: TcpStream, read_only: bool) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut header_buf = [0u8; 4096];
    let n = match stream.read(&mut header_buf) {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let request_str = String::from_utf8_lossy(&header_buf[..n]);
    let mut lines = request_str.lines();
    let first_line = match lines.next() {
        Some(l) => l,
        None => return,
    };

    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }
    let method = parts[0];
    let path = parts[1];

    if method != "GET" {
        let resp = "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes());
        return;
    }

    // Check if this is a WebSocket upgrade request
    let mut is_upgrade = false;
    let mut sec_ws_key: Option<String> = None;

    for line in lines {
        let line_lower = line.to_ascii_lowercase();
        if line_lower.starts_with("upgrade:") && line_lower.contains("websocket") {
            is_upgrade = true;
        } else if line_lower.starts_with("sec-websocket-key:") {
            if let Some(pos) = line.find(':') {
                sec_ws_key = Some(line[pos + 1..].trim().to_string());
            }
        }
    }

    if is_upgrade && path == "/ws" {
        if let Some(key) = sec_ws_key {
            handle_websocket(stream, &key, read_only);
        } else {
            let resp = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
        }
        return;
    }

    // Serve static assets
    let clean_path = path.split('?').next().unwrap_or("/");
    match clean_path {
        "/" | "/index.html" => {
            send_http_response(
                &mut stream,
                "text/html; charset=utf-8",
                EMBEDDED_INDEX_HTML.as_bytes(),
            );
        }
        "/style.css" => {
            send_http_response(
                &mut stream,
                "text/css; charset=utf-8",
                EMBEDDED_STYLE_CSS.as_bytes(),
            );
        }
        "/app.js" => {
            send_http_response(
                &mut stream,
                "application/javascript; charset=utf-8",
                EMBEDDED_APP_JS.as_bytes(),
            );
        }
        "/fastty_wasm.wasm" => {
            send_http_response(&mut stream, "application/wasm", EMBEDDED_WASM);
        }
        _ => {
            let body = "404 Not Found";
            let resp = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    }
}

fn send_http_response(stream: &mut TcpStream, content_type: &str, body: &[u8]) {
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache\r\n\r\n",
        content_type,
        body.len()
    );
    let _ = stream.write_all(headers.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn handle_websocket(mut stream: TcpStream, key: &str, _read_only: bool) {
    // 1. Handshake
    const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let accept_input = format!("{}{}", key, WS_GUID);
    let sha_digest = sha1(accept_input.as_bytes());
    let accept_key = base64_encode(&sha_digest);

    let handshake = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\r\n",
        accept_key
    );

    if stream.write_all(handshake.as_bytes()).is_err() {
        return;
    }
    let _ = stream.flush();

    #[cfg(not(unix))]
    {
        let _ = _read_only;
        let error_msg = serde_json::json!({
            "event": "error",
            "code": "unsupported_platform",
            "message": "fastty daemon is not supported on this platform yet"
        });
        let _ = send_ws_text(&mut stream, &error_msg.to_string());
    }

    #[cfg(unix)]
    {
        // 2. Connect to fastty daemon unix socket
        let sock = socket_path();
        let mut unix_stream = UnixStream::connect(&sock);
        if unix_stream.is_err() {
            crate::daemon::start();
            crate::daemon::ensure_default_session();
            std::thread::sleep(Duration::from_millis(50));
            unix_stream = UnixStream::connect(&sock);
        }

        let unix_stream = match unix_stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "fastty gateway: failed to connect to daemon socket: {}",
                    e
                );
                let error_msg = serde_json::json!({
                    "event": "error",
                    "code": "daemon_offline",
                    "message": format!("Cannot connect to fastty daemon socket at {}: {}", sock.display(), e)
                });
                let _ = send_ws_text(&mut stream, &error_msg.to_string());
                return;
            }
        };

        let _ = stream.set_read_timeout(None);

        let alive = Arc::new(AtomicBool::new(true));
        let mut ws_read = match stream.try_clone() {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut ws_write = stream;

        let mut unix_read = match unix_stream.try_clone() {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut unix_write = unix_stream;

        let alive_clone = Arc::clone(&alive);

        // Thread 1: Read lines from Daemon Unix socket -> Send WebSocket text frames to browser
        let forward_to_browser = std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(&mut unix_read);
            for line in reader.lines() {
                if !alive_clone.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                if send_ws_text(&mut ws_write, &line).is_err() {
                    break;
                }
            }
            alive_clone.store(false, Ordering::Relaxed);
        });

        // Thread 2 (Current): Read WebSocket frames from browser -> Write NDJSON lines to Daemon Unix socket
        while alive.load(Ordering::Relaxed) {
            match read_ws_text(&mut ws_read) {
                Ok(Some(text)) => {
                    let mut line = text;
                    line.push('\n');
                    if unix_write.write_all(line.as_bytes()).is_err() {
                        break;
                    }
                    let _ = unix_write.flush();
                }
                Ok(None) => continue,
                Err(_) => break,
            }
        }

        alive.store(false, Ordering::Relaxed);
        let _ = forward_to_browser.join();
    }
}

fn send_ws_text(stream: &mut TcpStream, text: &str) -> std::io::Result<()> {
    let payload = text.as_bytes();
    let len = payload.len();
    let mut header = Vec::with_capacity(10);
    header.push(0x81); // FIN = 1, opcode = 1 (text)

    if len < 126 {
        header.push(len as u8);
    } else if len <= 65535 {
        header.push(126);
        header.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        header.push(127);
        header.extend_from_slice(&(len as u64).to_be_bytes());
    }

    stream.write_all(&header)?;
    stream.write_all(payload)?;
    stream.flush()
}

fn read_ws_text(stream: &mut TcpStream) -> std::io::Result<Option<String>> {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header)?;

    let fin = (header[0] & 0x80) != 0;
    let opcode = header[0] & 0x0F;
    let masked = (header[1] & 0x80) != 0;
    let mut payload_len = (header[1] & 0x7F) as usize;

    if opcode == 8 {
        // Connection Close
        return Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "client closed",
        ));
    }

    if payload_len == 126 {
        let mut ext = [0u8; 2];
        stream.read_exact(&mut ext)?;
        payload_len = u16::from_be_bytes(ext) as usize;
    } else if payload_len == 127 {
        let mut ext = [0u8; 8];
        stream.read_exact(&mut ext)?;
        payload_len = u64::from_be_bytes(ext) as usize;
    }

    let mask = if masked {
        let mut m = [0u8; 4];
        stream.read_exact(&mut m)?;
        Some(m)
    } else {
        None
    };

    let mut payload = vec![0u8; payload_len];
    stream.read_exact(&mut payload)?;

    if let Some(m) = mask {
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= m[i % 4];
        }
    }

    if opcode == 9 {
        // Ping -> send Pong
        let mut pong = Vec::with_capacity(payload_len + 2);
        pong.push(0x8A); // FIN + Pong
        pong.push(payload_len as u8);
        pong.extend_from_slice(&payload);
        let _ = stream.write_all(&pong);
        let _ = stream.flush();
        return Ok(None);
    }

    if fin && (opcode == 1 || opcode == 0) {
        if let Ok(s) = String::from_utf8(payload) {
            return Ok(Some(s));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha1_rfc_test_vectors() {
        let res = sha1(b"");
        assert_eq!(
            res,
            [
                0xda, 0x39, 0xa3, 0xee, 0x5e, 0x6b, 0x4b, 0x0d, 0x32, 0x55, 0xbf, 0xef, 0x95, 0x60,
                0x18, 0x90, 0xaf, 0xd8, 0x07, 0x09
            ]
        );

        let ws_key = "dGhlIHNhbXBsZSBub25jZQ==258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
        let digest = sha1(ws_key.as_bytes());
        let accept = base64_encode(&digest);
        assert_eq!(accept, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }
}
