//! Kitty clipboard protocol (OSC 5522): multi-format clipboard access.
//!
//! Lets terminal programs read and write the system clipboard including
//! non-textual data (images), which is what Claude Code-style tools use for
//! image pasting. fastty answers `CSI ? 5522 $ p` with "supported" and
//! speaks the read/write packet protocol on OSC 5522. Spec:
//! https://sw.kovidgoyal.net/kitty/clipboard/
//! See docs/plans/ideas-superlogical.md item 14.

use base64::prelude::*;
use std::collections::HashMap;

/// Total bytes a single write transaction may accumulate. The spec floor
/// is "at least 64MB"; beyond this the write fails with EFBIG.
pub const MAX_WRITE_BYTES: usize = 64 * 1024 * 1024;
/// Raw bytes per DATA chunk in responses (spec: ≤4096 pre-encoding).
pub const MAX_DATA_CHUNK: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
pub enum ReadRequest {
    /// The client sent a bare `.` — announce the formats we can offer.
    ListFormats,
    /// Space-separated MIME list, e.g. `text/plain image/png`.
    Formats(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// `type=write` — begin a write transaction.
    WriteBegin { id: Option<String> },
    /// `type=wdata[:mime=<b64>]` — one decoded chunk. `mime: None` with an
    /// empty payload is the end-of-transmission packet.
    WriteData { id: Option<String>, mime: Option<String>, data: Vec<u8> },
    /// `type=read` — query the clipboard.
    Read { id: Option<String>, request: ReadRequest },
}

fn b64_decode_strict(bytes: &[u8]) -> Option<Vec<u8>> {
    let s = std::str::from_utf8(bytes).ok()?;
    BASE64_STANDARD.decode(s).ok()
}

/// Parses the metadata/payload halves of an `OSC 5522`. Returns None for
/// malformed packets (bad base64, unknown type): callers stay silent and
/// the client treats the timeout as unsupported.
pub fn parse(metadata: &str, payload: &[u8]) -> Option<Message> {
    let mut kind: Option<&str> = None;
    let mut id = None;
    let mut mime = None;
    for part in metadata.split(':') {
        let Some((key, val)) = part.split_once('=') else { continue };
        match key {
            "type" => kind = Some(val),
            "id" => id = Some(val.to_string()),
            "mime" => {
                let decoded = b64_decode_strict(val.as_bytes())?;
                mime = Some(String::from_utf8(decoded).ok()?);
            }
            _ => {}
        }
    }
    match kind? {
        "write" => Some(Message::WriteBegin { id }),
        "wdata" => match mime {
            Some(mime) => Some(Message::WriteData { id, mime: Some(mime), data: b64_decode_strict(payload)? }),
            None => {
                if payload.is_empty() {
                    Some(Message::WriteData { id, mime: None, data: Vec::new() })
                } else {
                    None
                }
            }
        },
        "read" => {
            let request = if payload == b"." {
                ReadRequest::ListFormats
            } else {
                let decoded = b64_decode_strict(payload)?;
                let list = String::from_utf8(decoded).ok()?;
                let formats = list
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                if formats.is_empty() {
                    ReadRequest::ListFormats
                } else {
                    ReadRequest::Formats(formats)
                }
            };
            Some(Message::Read { id, request })
        }
        _ => None,
    }
}

/// One `type=write` transaction, accumulating chunks per MIME type.
#[derive(Default)]
pub struct WriteSession {
    pub id: Option<String>,
    buffers: HashMap<String, Vec<u8>>,
    total: usize,
}

impl WriteSession {
    pub fn new(id: Option<String>) -> Self {
        Self { id, ..Default::default() }
    }

    /// Appends a chunk. Err(()) when the transaction exceeds
    /// [`MAX_WRITE_BYTES`]; the caller drops the session and replies EFBIG.
    pub fn push(&mut self, mime: String, data: Vec<u8>) -> Result<(), ()> {
        if self.total + data.len() > MAX_WRITE_BYTES {
            return Err(());
        }
        self.total += data.len();
        self.buffers.entry(mime).or_default().extend_from_slice(&data);
        Ok(())
    }

    /// Applies the transaction to the system clipboard. arboard holds one
    /// representation, so we prefer text and fall back to the first image.
    /// Err(()) maps to an EIO reply.
    pub fn finish(self) -> Result<(), ()> {
        let Some((mime, data)) = self.pick() else {
            return Ok(());
        };
        let mut clipboard = arboard::Clipboard::new().map_err(|_| ())?;
        if mime.starts_with("text/") {
            let text = String::from_utf8_lossy(&data).into_owned();
            clipboard.set_text(text).map_err(|_| ())
        } else {
            let img = image::load_from_memory(&data).map_err(|_| ())?.to_rgba8();
            let (width, height) = img.dimensions();
            clipboard
                .set_image(arboard::ImageData {
                    width: width as usize,
                    height: height as usize,
                    bytes: std::borrow::Cow::Owned(img.into_raw()),
                })
                .map_err(|_| ())
        }
    }

    fn pick(&self) -> Option<(String, Vec<u8>)> {
        if let Some(data) = self.buffers.get("text/plain") {
            return Some(("text/plain".to_string(), data.clone()));
        }
        if let Some((mime, data)) = self.buffers.iter().find(|(m, _)| m.starts_with("text/")) {
            return Some(((*mime).clone(), data.clone()));
        }
        if let Some((mime, data)) = self.buffers.iter().find(|(m, _)| m.starts_with("image/")) {
            return Some(((*mime).clone(), data.clone()));
        }
        self.buffers
            .iter()
            .next()
            .map(|(m, d)| (m.clone(), d.clone()))
    }
}

/// Reads the clipboard for a request, returning the (mime, bytes) pairs we
/// can actually supply, in request order. Images always come back as PNG.
pub fn read(request: &ReadRequest) -> Vec<(String, Vec<u8>)> {
    let Ok(mut clipboard) = arboard::Clipboard::new() else {
        return Vec::new();
    };
    let text = clipboard.get_text().ok().filter(|t| !t.is_empty());
    let png = clipboard
        .get_image()
        .ok()
        .and_then(|img| encode_png(&img));
    let wanted = match request {
        ReadRequest::ListFormats => None,
        ReadRequest::Formats(list) => Some(list),
    };
    let mut out = Vec::new();
    let offer_text = || text.clone().map(|t| ("text/plain".to_string(), t.into_bytes()));
    let offer_png = || png.clone().map(|b| ("image/png".to_string(), b));
    match wanted {
        None => {
            // Format listing: one empty DATA packet per available format.
            if text.is_some() {
                out.push(("text/plain".to_string(), Vec::new()));
            }
            if png.is_some() {
                out.push(("image/png".to_string(), Vec::new()));
            }
        }
        Some(list) => {
            for fmt in list {
                let got = if fmt.starts_with("text/") {
                    offer_text()
                } else if fmt.starts_with("image/") {
                    offer_png()
                } else {
                    None
                };
                if let Some(pair) = got {
                    out.push(pair);
                }
            }
        }
    }
    out
}

fn encode_png(img: &arboard::ImageData) -> Option<Vec<u8>> {
    let rgba = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.to_vec())?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    rgba.write_to(&mut bytes, image::ImageFormat::Png).ok()?;
    Some(bytes.into_inner())
}

/// Status-only response (`type=read:status=OK` and friends).
pub fn response(kind: &str, status: &str, id: Option<&str>) -> String {
    let mut meta = format!("type={kind}:status={status}");
    if let Some(id) = id {
        meta.push_str(&format!(":id={id}"));
    }
    format!("\x1b]5522;{meta}\x1b\\")
}

/// One DATA chunk of a read response, base64 on both halves.
pub fn response_data(mime: &str, chunk: &[u8], id: Option<&str>) -> String {
    let mut meta = format!(
        "type=read:status=DATA:mime={}",
        BASE64_STANDARD.encode(mime)
    );
    if let Some(id) = id {
        meta.push_str(&format!(":id={id}"));
    }
    format!("\x1b]5522;{meta};{}\x1b\\", BASE64_STANDARD.encode(chunk))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> String {
        BASE64_STANDARD.encode(s)
    }

    #[test]
    fn parse_write_flow() {
        let msg = parse("type=write", b"").unwrap();
        assert_eq!(msg, Message::WriteBegin { id: None });

        let mime = b64("text/plain");
        let meta = format!("type=wdata:mime={mime}");
        let data = b64("hola");
        let msg = parse(&meta, data.as_bytes()).unwrap();
        assert_eq!(
            msg,
            Message::WriteData {
                id: None,
                mime: Some("text/plain".to_string()),
                data: b"hola".to_vec(),
            }
        );

        // End of transmission: bare wdata with empty payload.
        let msg = parse("type=wdata", b"").unwrap();
        assert_eq!(msg, Message::WriteData { id: None, mime: None, data: Vec::new() });
    }

    #[test]
    fn parse_read_formats_and_list() {
        let payload = b64("text/plain image/png");
        let msg = parse("type=read", payload.as_bytes()).unwrap();
        assert_eq!(
            msg,
            Message::Read {
                id: None,
                request: ReadRequest::Formats(vec![
                    "text/plain".to_string(),
                    "image/png".to_string()
                ]),
            }
        );

        let msg = parse("type=read", b".").unwrap();
        assert_eq!(msg, Message::Read { id: None, request: ReadRequest::ListFormats });

        let id = "abc-1".to_string();
        let msg = parse("type=read:id=abc-1", b".").unwrap();
        assert_eq!(msg, Message::Read { id: Some(id), request: ReadRequest::ListFormats });
    }

    #[test]
    fn parse_rejects_bad_base64() {
        assert!(parse("type=wdata:mime=!!!", b"aGk=").is_none());
        assert!(parse("type=read", b"not-base64!!").is_none());
        assert!(parse("type=banana", b".").is_none());
    }

    #[test]
    fn write_session_accumulates_and_enforces_cap() {
        let mut s = WriteSession::new(None);
        s.push("text/plain".to_string(), b"hello ".to_vec()).unwrap();
        s.push("text/plain".to_string(), b"world".to_vec()).unwrap();
        let (mime, data) = s.pick().unwrap();
        assert_eq!(mime, "text/plain");
        assert_eq!(data, b"hello world");

        let mut s = WriteSession::new(None);
        let big = vec![0u8; MAX_WRITE_BYTES - 4];
        s.push("image/png".to_string(), big).unwrap();
        assert!(s.push("image/png".to_string(), b"12345".to_vec()).is_err());
    }

    #[test]
    fn response_formats_match_spec_shapes() {
        assert_eq!(response("read", "OK", None), "\x1b]5522;type=read:status=OK\x1b\\");
        assert_eq!(
            response("write", "DONE", Some("x1")),
            "\x1b]5522;type=write:status=DONE:id=x1\x1b\\"
        );
        let got = response_data("text/plain", b"A", None);
        assert_eq!(got, "\x1b]5522;type=read:status=DATA:mime=dGV4dC9wbGFpbg==;QQ==\x1b\\");
    }
}
