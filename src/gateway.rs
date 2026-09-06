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
use std::sync::Arc;
use std::time::Duration;

use crate::daemon::{base64_encode, socket_path};

const EMBEDDED_INDEX_HTML: &str = include_str!("../web/index.html");
const EMBEDDED_STYLE_CSS: &str = include_str!("../web/style.css");
const EMBEDDED_APP_JS: &str = include_str!("../web/app.js");
const EMBEDDED_PKG_JS: &str = include_str!("../web/pkg/fastty_wasm.js");
const EMBEDDED_PKG_WASM: &[u8] = include_bytes!("../web/pkg/fastty_wasm_bg.wasm");

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

/// Check if a host address is a local loopback address.
pub fn is_loopback_host(host: &str) -> bool {
    let clean = host.trim().trim_start_matches('[').trim_end_matches(']');
    if clean.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = clean.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    false
}

/// Generate a cryptographically random 32-character hex token.
pub fn generate_random_token() -> String {
    #[cfg(unix)]
    {
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let mut buf = [0u8; 16];
            if f.read_exact(&mut buf).is_ok() {
                return buf.iter().map(|b| format!("{:02x}", b)).collect();
            }
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let seed = format!("{}:{}:fastty_gateway_token", now, pid);
    let h1 = seahash::hash(seed.as_bytes());
    let h2 = seahash::hash(&h1.to_le_bytes());
    format!("{:016x}{:016x}", h1, h2)
}

/// Extract a query parameter by key from an HTTP request path.
pub fn extract_query_param(path: &str, param: &str) -> Option<String> {
    let query = path.split_once('?')?;
    for pair in query.1.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == param {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Query the Resident Set Size (RSS) of the current process in bytes.
#[allow(deprecated)]
pub fn get_process_rss_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        use std::mem;
        let mut info: libc::mach_task_basic_info = unsafe { mem::zeroed() };
        let mut count = (mem::size_of::<libc::mach_task_basic_info>()
            / mem::size_of::<libc::natural_t>()) as libc::mach_msg_type_number_t;
        let kerr = unsafe {
            libc::task_info(
                libc::mach_task_self(),
                libc::MACH_TASK_BASIC_INFO,
                &mut info as *mut _ as *mut libc::integer_t,
                &mut count,
            )
        };
        if kerr == libc::KERN_SUCCESS {
            Some(info.resident_size)
        } else {
            None
        }
    }
    #[cfg(target_os = "linux")]
    {
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let mut parts = statm.split_whitespace();
        let _size = parts.next()?;
        let resident_pages: u64 = parts.next()?.parse().ok()?;
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page_size > 0 {
            Some(resident_pages * page_size as u64)
        } else {
            Some(resident_pages * 4096)
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Format bytes into human-readable MiB string.
pub fn format_rss_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    format!("{:.1} MiB", mb)
}

fn parse_host_port(authority: &str) -> (&str, Option<&str>) {
    if authority.starts_with('[') {
        if let Some(end_bracket) = authority.find(']') {
            let h = &authority[..=end_bracket];
            let rest = &authority[end_bracket + 1..];
            let p = rest.strip_prefix(':');
            return (h, p);
        }
    }
    match authority.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    }
}

/// Validate that the Origin header matches the Host request header.
pub fn is_origin_allowed(origin: &str, host: &str) -> bool {
    let origin_trimmed = origin.trim();
    let host_trimmed = host.trim();

    if origin_trimmed.eq_ignore_ascii_case("null") {
        return false;
    }
    if host_trimmed.is_empty() {
        return false;
    }

    let origin_authority = if let Some(idx) = origin_trimmed.find("://") {
        &origin_trimmed[idx + 3..]
    } else {
        origin_trimmed
    };
    let origin_authority = origin_authority.split('/').next().unwrap_or("").trim();

    if origin_authority.eq_ignore_ascii_case(host_trimmed) {
        return true;
    }

    let (o_host, o_port) = parse_host_port(origin_authority);
    let (h_host, h_port) = parse_host_port(host_trimmed);

    if !o_host.eq_ignore_ascii_case(h_host) {
        return false;
    }

    match (o_port, h_port) {
        (Some(p1), Some(p2)) => p1 == p2,
        (None, None) => true,
        _ => false,
    }
}

pub fn run_gateway(host: &str, port: u16, read_only: bool, token: Option<String>) {
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

    let token_required = token.is_some() || !is_loopback_host(host);
    let auth_token = if token_required {
        Some(token.unwrap_or_else(generate_random_token))
    } else {
        None
    };

    println!("\n  🚀 Fastty Web Gateway");
    let display_host = if host == "0.0.0.0" { "localhost" } else { host };
    if let Some(ref tok) = auth_token {
        println!(
            "  ➜ Local:   http://{}:{}?token={}",
            display_host, port, tok
        );
        println!("  ➜ Token:   {}", tok);
    } else {
        println!(
            "  ➜ Local:   http://{}:{}",
            display_host, port
        );
    }
    println!("  ➜ Daemon:  {}", socket_path().display());
    if read_only {
        println!("  ➜ Mode:    read-only (writes ignored)");
    }
    if let Some(rss) = get_process_rss_bytes() {
        println!("  ➜ RSS:     {}", format_rss_bytes(rss));
    }
    println!("  ➜ Ready for browser connections. Press Ctrl+C to stop.\n");

    let auth_token_arc = Arc::new(auth_token);
    for stream in listener.incoming().flatten() {
        let auth_token = Arc::clone(&auth_token_arc);
        std::thread::spawn(move || {
            handle_client(stream, read_only, auth_token.as_deref());
        });
    }
}

fn handle_client(mut stream: TcpStream, read_only: bool, auth_token: Option<&str>) {
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

    // Check if this is a WebSocket upgrade request and parse headers
    let mut is_upgrade = false;
    let mut sec_ws_key: Option<String> = None;
    let mut host_header: Option<String> = None;
    let mut origin_header: Option<String> = None;

    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            if k == "upgrade" && v.to_ascii_lowercase().contains("websocket") {
                is_upgrade = true;
            } else if k == "sec-websocket-key" {
                sec_ws_key = Some(v.to_string());
            } else if k == "host" {
                host_header = Some(v.to_string());
            } else if k == "origin" {
                origin_header = Some(v.to_string());
            }
        }
    }

    let clean_path = path.split('?').next().unwrap_or("/");

    if is_upgrade && clean_path == "/ws" {
        // Origin validation: reject if Origin present and differs from Host
        if let Some(ref origin) = origin_header {
            let host = host_header.as_deref().unwrap_or("");
            if !is_origin_allowed(origin, host) {
                let body = "403 Forbidden: Cross-origin WebSocket upgrade rejected";
                let resp = format!(
                    "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                return;
            }
        }

        // Access token validation: mandatory if token configured
        if let Some(expected_token) = auth_token {
            let provided = extract_query_param(path, "token");
            if provided.as_deref() != Some(expected_token) {
                let body = "401 Unauthorized: Invalid or missing access token";
                let resp = format!(
                    "HTTP/1.1 401 Unauthorized\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                return;
            }
        }

        if let Some(key) = sec_ws_key {
            handle_websocket(stream, &key, read_only);
        } else {
            let resp = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
        }
        return;
    }

    // Serve static assets
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
        "/pkg/fastty_wasm.js" => {
            send_http_response(
                &mut stream,
                "application/javascript; charset=utf-8",
                EMBEDDED_PKG_JS.as_bytes(),
            );
        }
        "/pkg/fastty_wasm_bg.wasm" | "/fastty_wasm_bg.wasm" | "/fastty_wasm.wasm" => {
            send_http_response(&mut stream, "application/wasm", EMBEDDED_PKG_WASM);
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

fn handle_websocket(mut stream: TcpStream, key: &str, read_only: bool) {
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
        let _ = read_only;
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

        let mut unix_stream = match unix_stream {
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

        // When in read_only mode, send hello with read_only flag to enforce on daemon side
        if read_only {
            let hello_req = serde_json::to_string(&crate::daemon::Request::Hello { read_only: true })
                .unwrap_or_else(|_| r#"{"cmd":"hello","read_only":true}"#.to_string());
            let _ = unix_stream.write_all(format!("{}\n", hello_req).as_bytes());
            let _ = unix_stream.flush();
        }

        let _ = stream.set_read_timeout(None);

        let alive = Arc::new(AtomicBool::new(true));
        let mut ws_read = match stream.try_clone() {
            Ok(s) => s,
            Err(_) => return,
        };
        let ws_write = Arc::new(parking_lot::Mutex::new(stream));

        let mut unix_read = match unix_stream.try_clone() {
            Ok(s) => s,
            Err(_) => return,
        };
        let mut unix_write = unix_stream;

        let alive_clone = Arc::clone(&alive);
        let ws_write_for_browser = Arc::clone(&ws_write);

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
                if line.contains("\"event\":\"binary_snapshot\"") {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                        if let Some(b64) = val.get("data").and_then(|d| d.as_str()) {
                            if let Some(bin) = crate::daemon::base64_decode(b64) {
                                if send_ws_binary(&mut *ws_write_for_browser.lock(), &bin).is_err() {
                                    break;
                                }
                                continue;
                            }
                        }
                    }
                }
                if send_ws_text(&mut *ws_write_for_browser.lock(), &line).is_err() {
                    break;
                }
            }
            alive_clone.store(false, Ordering::Relaxed);
        });

        // Thread 2 (Current): Read WebSocket frames from browser -> Write NDJSON lines to Daemon Unix socket
        while alive.load(Ordering::Relaxed) {
            match read_ws_text(&mut ws_read) {
                Ok(Some(text)) => {
                    // Gateway bridge filtering: drop mutating commands in read-only mode
                    if read_only {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(cmd) = val.get("cmd").and_then(|c| c.as_str()) {
                                if matches!(cmd, "write" | "spawn" | "close") {
                                    let err_resp = serde_json::json!({
                                        "event": "error",
                                        "code": "read_only",
                                        "message": format!("command '{}' rejected: gateway is running in read-only mode", cmd)
                                    });
                                    let _ = send_ws_text(&mut *ws_write.lock(), &err_resp.to_string());
                                    continue;
                                }
                            }
                        }
                    }
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

pub fn send_ws_text(stream: &mut TcpStream, text: &str) -> std::io::Result<()> {
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

pub fn send_ws_binary(stream: &mut TcpStream, data: &[u8]) -> std::io::Result<()> {
    let len = data.len();
    let mut header = Vec::with_capacity(10);
    header.push(0x82); // FIN = 1, opcode = 2 (binary)

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
    stream.write_all(data)?;
    stream.flush()
}

pub const MAX_WS_FRAME_SIZE: usize = 1024 * 1024; // 1 MiB

pub fn send_ws_close_1009(stream: &mut TcpStream) -> std::io::Result<()> {
    let reason = b"frame exceeds 1 MiB limit";
    let payload_len = 2 + reason.len();
    let mut frame = Vec::with_capacity(2 + payload_len);
    frame.push(0x88); // FIN = 1, opcode = 8 (close)
    frame.push(payload_len as u8);
    frame.extend_from_slice(&1009u16.to_be_bytes());
    frame.extend_from_slice(reason);
    stream.write_all(&frame)?;
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

    if payload_len > MAX_WS_FRAME_SIZE {
        let _ = send_ws_close_1009(stream);
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds 1 MiB limit",
        ));
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

    #[test]
    fn test_is_loopback_host() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("LOCALHOST"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("127.0.0.2"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("[::1]"));

        assert!(!is_loopback_host("0.0.0.0"));
        assert!(!is_loopback_host("::"));
        assert!(!is_loopback_host("192.168.1.100"));
        assert!(!is_loopback_host("100.64.0.1"));
        assert!(!is_loopback_host("fastty.lan"));
    }

    #[test]
    fn test_is_origin_allowed() {
        // Matching hosts with ports
        assert!(is_origin_allowed("http://localhost:8765", "localhost:8765"));
        assert!(is_origin_allowed("https://localhost:8765", "localhost:8765"));
        assert!(is_origin_allowed("http://127.0.0.1:8765", "127.0.0.1:8765"));
        assert!(is_origin_allowed("http://[::1]:8765", "[::1]:8765"));
        assert!(is_origin_allowed("http://mybox.tailscale.net:8765", "mybox.tailscale.net:8765"));

        // Matching without port
        assert!(is_origin_allowed("http://example.com", "example.com"));

        // Cross-origin mismatches
        assert!(!is_origin_allowed("http://evil.com:8765", "localhost:8765"));
        assert!(!is_origin_allowed("http://evil.com", "localhost:8765"));
        assert!(!is_origin_allowed("http://localhost:3000", "localhost:8765"));
        assert!(!is_origin_allowed("null", "localhost:8765"));
        assert!(!is_origin_allowed("http://192.168.1.50:8765", "192.168.1.51:8765"));
    }

    #[test]
    fn test_extract_query_param() {
        assert_eq!(
            extract_query_param("/ws?token=secret123", "token"),
            Some("secret123".to_string())
        );
        assert_eq!(
            extract_query_param("/ws?foo=bar&token=abc_456&baz=1", "token"),
            Some("abc_456".to_string())
        );
        assert_eq!(extract_query_param("/ws", "token"), None);
        assert_eq!(extract_query_param("/ws?other=123", "token"), None);
    }

    #[test]
    fn test_handle_client_origin_rejection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                handle_client(stream, false, None);
            }
        });

        let mut client = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        let req = format!(
            "GET /ws HTTP/1.1\r\n\
             Host: 127.0.0.1:{}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Origin: http://evil.com\r\n\r\n",
            port
        );
        client.write_all(req.as_bytes()).unwrap();

        let mut resp = [0u8; 1024];
        let n = client.read(&mut resp).unwrap();
        let resp_str = String::from_utf8_lossy(&resp[..n]);
        assert!(resp_str.starts_with("HTTP/1.1 403 Forbidden"), "expected 403, got: {}", resp_str);
    }

    #[test]
    fn test_handle_client_token_enforcement() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            // First client: missing token
            if let Ok((stream, _)) = listener.accept() {
                handle_client(stream, false, Some("secret_token_123"));
            }
            // Second client: wrong token
            if let Ok((stream, _)) = listener.accept() {
                handle_client(stream, false, Some("secret_token_123"));
            }
        });

        // 1. Missing token -> 401
        let mut client1 = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        let req1 = format!(
            "GET /ws HTTP/1.1\r\n\
             Host: 127.0.0.1:{}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
            port
        );
        client1.write_all(req1.as_bytes()).unwrap();
        let mut resp1 = [0u8; 1024];
        let n1 = client1.read(&mut resp1).unwrap();
        let resp1_str = String::from_utf8_lossy(&resp1[..n1]);
        assert!(resp1_str.starts_with("HTTP/1.1 401 Unauthorized"), "expected 401, got: {}", resp1_str);

        // 2. Wrong token -> 401
        let mut client2 = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        let req2 = format!(
            "GET /ws?token=wrong_token HTTP/1.1\r\n\
             Host: 127.0.0.1:{}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
            port
        );
        client2.write_all(req2.as_bytes()).unwrap();
        let mut resp2 = [0u8; 1024];
        let n2 = client2.read(&mut resp2).unwrap();
        let resp2_str = String::from_utf8_lossy(&resp2[..n2]);
        assert!(resp2_str.starts_with("HTTP/1.1 401 Unauthorized"), "expected 401, got: {}", resp2_str);
    }

    #[test]
    fn test_oversize_ws_frame_rejection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            if let Ok((mut server_stream, _)) = listener.accept() {
                let res = read_ws_text(&mut server_stream);
                assert!(res.is_err(), "expected error for oversize frame");
            }
        });

        let mut client = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        let len: u64 = 1024 * 1024 + 10;
        let mut frame_header = Vec::new();
        frame_header.push(0x81); // FIN = 1, opcode = 1 (text), unmasked
        frame_header.push(127);  // 8-byte length indicator
        frame_header.extend_from_slice(&len.to_be_bytes());

        client.write_all(&frame_header).unwrap();

        let mut close_resp = [0u8; 64];
        let n = client.read(&mut close_resp).unwrap_or(0);
        assert!(n >= 4, "expected at least 4 bytes in close frame, got {}", n);
        assert_eq!(close_resp[0], 0x88); // Close opcode
        assert_eq!(close_resp[2], 0x03); // Status code 1009 high byte
        assert_eq!(close_resp[3], 0xF1); // Status code 1009 low byte
    }

    #[cfg(unix)]
    #[test]
    fn test_gateway_read_only_bridge_filtering() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                handle_client(stream, true, None);
            }
        });

        let mut client = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        let upgrade_req = format!(
            "GET /ws HTTP/1.1\r\n\
             Host: 127.0.0.1:{}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
            port
        );
        client.write_all(upgrade_req.as_bytes()).unwrap();

        let mut resp = [0u8; 1024];
        let n = client.read(&mut resp).unwrap();
        let resp_str = String::from_utf8_lossy(&resp[..n]);
        assert!(resp_str.starts_with("HTTP/1.1 101 Switching Protocols"));

        // Send a masked WS text frame with {"cmd":"spawn","cols":80,"rows":24}
        let payload = r#"{"cmd":"spawn","cols":80,"rows":24}"#.as_bytes();
        let mut frame = Vec::new();
        frame.push(0x81); // FIN = 1, opcode = 1
        frame.push(0x80 | (payload.len() as u8)); // Masked bit set
        let mask = [0x12, 0x34, 0x56, 0x78];
        frame.extend_from_slice(&mask);
        for (i, &b) in payload.iter().enumerate() {
            frame.push(b ^ mask[i % 4]);
        }
        client.write_all(&frame).unwrap();

        // Read response frames from gateway (may receive daemon hello response first)
        let mut found_read_only_error = false;
        for _ in 0..5 {
            if let Ok(Some(msg)) = read_ws_text(&mut client) {
                if msg.contains(r#""code":"read_only""#) {
                    found_read_only_error = true;
                    break;
                }
            }
        }
        assert!(found_read_only_error, "expected read_only error response from gateway");
    }
}
