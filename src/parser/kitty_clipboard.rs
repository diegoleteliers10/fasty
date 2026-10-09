//! Kitty clipboard protocol (OSC 5522): multi-format clipboard access.
//!
//! Lets terminal programs read and write the system clipboard including
//! non-textual data (images), which is what Claude Code-style tools use for
//! image pasting. fastty answers `CSI ? 5522 $ p` with "supported" and
//! speaks the read/write packet protocol on OSC 5522. Spec:
//! https://sw.kovidgoyal.net/kitty/clipboard/
//! See docs/plans/ideas-superlogical.md item 14.

use base64::prelude::*;
use std::collections::BTreeMap;

/// Total bytes a single write transaction may accumulate. The spec floor
/// is "at least 64MB"; beyond this the write fails with EFBIG.
pub const MAX_WRITE_BYTES: usize = 64 * 1024 * 1024;
/// Raw bytes per DATA chunk in responses (spec: ≤4096 pre-encoding).
pub const MAX_DATA_CHUNK: usize = 4096;
const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;

/// Read requests fail when the requested selection is unavailable or the
/// clipboard backend cannot provide the requested format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadError {
    UnsupportedLocation,
    UnsupportedFormat,
    BackendUnavailable,
}

impl ReadError {
    /// Returns the protocol status for this read failure.
    pub fn status(self) -> &'static str {
        match self {
            Self::UnsupportedLocation | Self::UnsupportedFormat => "ENOSYS",
            Self::BackendUnavailable => "EBUSY",
        }
    }
}

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
    WriteData {
        id: Option<String>,
        mime: Option<String>,
        data: Vec<u8>,
    },
    /// `type=read` — query the clipboard.
    Read {
        id: Option<String>,
        request: ReadRequest,
    },
}

/// Clipboard location requested by an application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Clipboard,
    Primary,
}

/// Parsed request with metadata needed by the full protocol.
#[derive(Debug, Clone, PartialEq)]
pub enum ProtocolMessage {
    WriteBegin {
        id: Option<String>,
        location: Location,
        password: Option<String>,
        name: Option<String>,
    },
    WriteData {
        id: Option<String>,
        mime: Option<String>,
        data: Vec<u8>,
    },
    /// Base64 bytes that must be joined with later packets for this MIME type
    /// before decoding. OSC 5522 permits packet boundaries inside a quartet.
    WriteDataEncoded {
        id: Option<String>,
        mime: String,
        data: Vec<u8>,
    },
    WriteAlias {
        mime: String,
        aliases: Vec<String>,
    },
    Read {
        id: Option<String>,
        request: ReadRequest,
        location: Location,
        password: Option<String>,
        name: Option<String>,
    },
}

fn b64_decode_strict(bytes: &[u8]) -> Option<Vec<u8>> {
    let s = std::str::from_utf8(bytes).ok()?;
    BASE64_STANDARD.decode(s).ok()
}

/// Parses the metadata/payload halves of an `OSC 5522`. Returns None for
/// malformed packets (bad base64, unknown type): callers stay silent and
/// the client treats the timeout as unsupported.
pub fn parse(metadata: &str, payload: &[u8]) -> Option<Message> {
    match parse_protocol(metadata, payload)? {
        ProtocolMessage::WriteBegin { id, .. } => Some(Message::WriteBegin { id }),
        ProtocolMessage::WriteData { id, mime, data } => {
            Some(Message::WriteData { id, mime, data })
        }
        ProtocolMessage::WriteDataEncoded { id, mime, data } => Some(Message::WriteData {
            id,
            mime: Some(mime),
            data: b64_decode_strict(&data)?,
        }),
        ProtocolMessage::Read { id, request, .. } => Some(Message::Read { id, request }),
        ProtocolMessage::WriteAlias { .. } => None,
    }
}

/// Parses OSC 5522 while retaining location and authorization metadata.
/// Invalid identifiers are filtered as required before any response echoes them.
pub fn parse_protocol(metadata: &str, payload: &[u8]) -> Option<ProtocolMessage> {
    let mut kind: Option<&str> = None;
    let mut id: Option<String> = None;
    let mut mime: Option<String> = None;
    let mut location = Location::Clipboard;
    let mut password = None;
    let mut name = None;
    for part in metadata.split(':') {
        let Some((key, val)) = part.split_once('=') else {
            continue;
        };
        match key {
            "type" => kind = Some(val),
            "id" => id = Some(sanitize_id(val)),
            "loc" => {
                location = match val {
                    "clipboard" => Location::Clipboard,
                    "primary" => Location::Primary,
                    _ => return None,
                }
            }
            "mime" => mime = Some(decode_text(val)?),
            "pw" => password = Some(decode_text(val)?),
            "name" => name = Some(decode_text(val)?),
            _ => {}
        }
    }
    match kind? {
        "write" => Some(ProtocolMessage::WriteBegin {
            id,
            location,
            password,
            name,
        }),
        "wdata" => match mime {
            Some(mime) => {
                if payload.len() > 5464 || !valid_base64_segment(payload) {
                    return None;
                }
                Some(ProtocolMessage::WriteDataEncoded {
                    id,
                    mime,
                    data: payload.to_vec(),
                })
            }
            None => {
                if payload.is_empty() {
                    Some(ProtocolMessage::WriteData {
                        id,
                        mime: None,
                        data: Vec::new(),
                    })
                } else {
                    None
                }
            }
        },
        "walias" => {
            let mime = mime?;
            let decoded = b64_decode_strict(payload)?;
            let aliases = String::from_utf8(decoded)
                .ok()?
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if aliases.is_empty() {
                return None;
            }
            Some(ProtocolMessage::WriteAlias { mime, aliases })
        }
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
            Some(ProtocolMessage::Read {
                id,
                request,
                location,
                password,
                name,
            })
        }
        _ => None,
    }
}

fn decode_text(encoded: &str) -> Option<String> {
    String::from_utf8(b64_decode_strict(encoded.as_bytes())?).ok()
}

fn sanitize_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '.'))
        .collect()
}

fn valid_base64_segment(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
}

/// One `type=write` transaction, accumulating chunks per MIME type.
#[derive(Default)]
pub struct WriteSession {
    pub id: Option<String>,
    buffers: BTreeMap<String, Vec<u8>>,
    encoded_buffers: BTreeMap<String, Vec<u8>>,
    aliases: BTreeMap<String, Vec<String>>,
    total: usize,
}

impl WriteSession {
    pub fn new(id: Option<String>) -> Self {
        Self {
            id,
            ..Default::default()
        }
    }

    /// Appends a chunk. Err(()) when the transaction exceeds
    /// [`MAX_WRITE_BYTES`]; the caller drops the session and replies EFBIG.
    pub fn push(&mut self, mime: String, data: Vec<u8>) -> Result<(), ()> {
        if self.total.saturating_add(data.len()) > MAX_WRITE_BYTES {
            return Err(());
        }
        self.total += data.len();
        self.buffers
            .entry(mime)
            .or_default()
            .extend_from_slice(&data);
        Ok(())
    }

    /// Appends an encoded packet payload. The session decodes all packets for
    /// a MIME type as one base64 stream when it finishes.
    pub fn push_encoded(&mut self, mime: String, data: Vec<u8>) -> Result<(), ()> {
        let encoded_total = self.encoded_buffers.values().map(Vec::len).sum::<usize>();
        let encoded_limit = MAX_WRITE_BYTES
            .saturating_mul(4)
            .saturating_div(3)
            .saturating_add(4);
        if data.len() > 5464
            || !valid_base64_segment(&data)
            || encoded_total.saturating_add(data.len()) > encoded_limit
        {
            return Err(());
        }
        self.encoded_buffers.entry(mime).or_default().extend(data);
        Ok(())
    }

    /// Adds MIME aliases to the representation supplied by `mime`.
    pub fn add_aliases(&mut self, mime: String, aliases: Vec<String>) -> Result<(), ()> {
        if aliases.is_empty() || mime.is_empty() || aliases.iter().any(|alias| alias.is_empty()) {
            return Err(());
        }
        self.aliases.insert(mime, aliases);
        Ok(())
    }

    /// Returns formats supplied by the transaction, including protocol aliases.
    pub fn formats(&self) -> Vec<String> {
        let mut formats = self.buffers.keys().cloned().collect::<Vec<_>>();
        formats.extend(self.encoded_buffers.keys().cloned());
        for aliases in self.aliases.values() {
            formats.extend(aliases.iter().cloned());
        }
        formats.sort();
        formats.dedup();
        formats
    }

    /// Applies the transaction to the system clipboard. arboard holds one
    /// representation, so we prefer text and fall back to the first image.
    /// Err(()) maps to an EIO reply.
    pub fn finish(self) -> Result<(), ()> {
        let mut session = self;
        for (mime, encoded) in std::mem::take(&mut session.encoded_buffers) {
            let data = b64_decode_strict(&encoded).ok_or(())?;
            if session.total.saturating_add(data.len()) > MAX_WRITE_BYTES {
                return Err(());
            }
            session.total += data.len();
            session.buffers.entry(mime).or_default().extend(data);
        }
        let Some((mime, data)) = session.pick() else {
            return Ok(());
        };
        let mut clipboard = arboard::Clipboard::new().map_err(|_| ())?;
        if mime.eq_ignore_ascii_case("text/plain")
            || mime.eq_ignore_ascii_case("text/plain;charset=utf-8")
        {
            let text = String::from_utf8(data).map_err(|_| ())?;
            clipboard.set_text(text).map_err(|_| ())
        } else if mime.to_ascii_lowercase().starts_with("image/") {
            let mut reader = image::ImageReader::new(std::io::Cursor::new(&data))
                .with_guessed_format()
                .map_err(|_| ())?;
            reader.limits(image::Limits {
                max_image_width: Some(MAX_IMAGE_DIMENSION),
                max_image_height: Some(MAX_IMAGE_DIMENSION),
                max_alloc: Some(MAX_IMAGE_PIXELS * 4),
            });
            let img = reader.decode().map_err(|_| ())?.to_rgba8();
            let (width, height) = img.dimensions();
            if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
                return Err(());
            }
            clipboard
                .set_image(arboard::ImageData {
                    width: width as usize,
                    height: height as usize,
                    bytes: std::borrow::Cow::Owned(img.into_raw()),
                })
                .map_err(|_| ())
        } else {
            Err(())
        }
    }

    fn pick(&self) -> Option<(String, Vec<u8>)> {
        if let Some(data) = self.buffers.get("text/plain") {
            return Some(("text/plain".to_string(), data.clone()));
        }
        if let Some((mime, data)) = self.buffers.iter().find(|(m, _)| {
            m.eq_ignore_ascii_case("text/plain")
                || m.eq_ignore_ascii_case("text/plain;charset=utf-8")
        }) {
            return Some(((*mime).clone(), data.clone()));
        }
        if let Some((mime, data)) = self
            .buffers
            .iter()
            .find(|(m, _)| m.to_ascii_lowercase().starts_with("image/"))
        {
            return Some(((*mime).clone(), data.clone()));
        }
        for (source, aliases) in &self.aliases {
            if let Some(data) = self.buffers.get(source) {
                if aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case("text/plain"))
                {
                    return Some(("text/plain".to_string(), data.clone()));
                }
                if aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case("image/png"))
                {
                    return Some(("image/png".to_string(), data.clone()));
                }
            }
        }
        None
    }
}

/// Reads the clipboard for a request, returning the (mime, bytes) pairs we
/// can actually supply, in request order. Images always come back as PNG.
pub fn read(request: &ReadRequest) -> Vec<(String, Vec<u8>)> {
    read_from_location(request, Location::Clipboard).unwrap_or_default()
}

/// Reads formats supported by Fastty from the requested selection.
/// Fastty currently has no primary-selection backend.
pub fn read_from_location(
    request: &ReadRequest,
    location: Location,
) -> Result<Vec<(String, Vec<u8>)>, ReadError> {
    if location == Location::Primary {
        return Err(ReadError::UnsupportedLocation);
    }
    let mut clipboard = arboard::Clipboard::new().map_err(|_| ReadError::BackendUnavailable)?;
    let text = clipboard.get_text().ok();
    let png = clipboard.get_image().ok().and_then(|img| encode_png(&img));
    let wanted = match request {
        ReadRequest::ListFormats => None,
        ReadRequest::Formats(list) => Some(list),
    };
    let mut out = Vec::new();
    let offer_text = || {
        text.clone()
            .map(|t| ("text/plain".to_string(), t.into_bytes()))
    };
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
            let mut seen = std::collections::BTreeSet::new();
            for fmt in list {
                let normalized = fmt.to_ascii_lowercase();
                if !seen.insert(normalized.clone()) {
                    continue;
                }
                let got = if normalized == "text/plain" || normalized == "text/plain;charset=utf-8"
                {
                    offer_text()
                } else if normalized == "image/png" {
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
    if matches!(wanted, Some(_)) && out.is_empty() {
        Err(ReadError::UnsupportedFormat)
    } else {
        Ok(out)
    }
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
        meta.push_str(&format!(":id={}", sanitize_id(id)));
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
        meta.push_str(&format!(":id={}", sanitize_id(id)));
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
        assert_eq!(
            msg,
            Message::WriteData {
                id: None,
                mime: None,
                data: Vec::new()
            }
        );
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
        assert_eq!(
            msg,
            Message::Read {
                id: None,
                request: ReadRequest::ListFormats
            }
        );

        let id = "abc-1".to_string();
        let msg = parse("type=read:id=abc-1", b".").unwrap();
        assert_eq!(
            msg,
            Message::Read {
                id: Some(id),
                request: ReadRequest::ListFormats
            }
        );
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
        s.push("text/plain".to_string(), b"hello ".to_vec())
            .unwrap();
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
        assert_eq!(
            response("read", "OK", None),
            "\x1b]5522;type=read:status=OK\x1b\\"
        );
        assert_eq!(
            response("write", "DONE", Some("x1")),
            "\x1b]5522;type=write:status=DONE:id=x1\x1b\\"
        );
        let got = response_data("text/plain", b"A", None);
        assert_eq!(
            got,
            "\x1b]5522;type=read:status=DATA:mime=dGV4dC9wbGFpbg==;QQ==\x1b\\"
        );
    }
}
