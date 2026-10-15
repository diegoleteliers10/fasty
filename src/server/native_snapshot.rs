use alacritty_terminal::snapshot::TerminalSnapshot;
use alacritty_terminal::vte::ansi::ProcessorSnapshot;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct NativeState {
    #[serde(flatten)]
    pub terminal: TerminalSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<Continuation>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Continuation {
    pub parser: ProcessorSnapshot,
    pub side: crate::terminal_state::SideCheckpoint,
}

impl NativeState {
    fn validate(&self) -> Result<(), String> {
        self.terminal
            .validate()
            .map_err(|error| error.to_string())?;
        if let Some(continuation) = &self.continuation {
            continuation.parser.validate().map_err(str::to_string)?;
            continuation.side.validate()?;
        }
        Ok(())
    }
}

pub const MAGIC: &[u8; 4] = b"FST2";
const VERSION: u16 = 1;
const HEADER_LEN: usize = 12;
const MAX_PAYLOAD: usize = 256 * 1024 * 1024;

#[derive(Clone, Copy)]
pub enum Encoding {
    Raw,
    Deflate,
}

pub fn encode(snapshot: &TerminalSnapshot, encoding: Encoding) -> Result<Vec<u8>, String> {
    encode_state(
        &NativeState {
            terminal: snapshot.clone(),
            continuation: None,
        },
        encoding,
    )
}

pub fn encode_state(snapshot: &NativeState, encoding: Encoding) -> Result<Vec<u8>, String> {
    snapshot.validate()?;
    let payload = serde_json::to_vec(snapshot).map_err(|error| error.to_string())?;
    if payload.len() > MAX_PAYLOAD {
        return Err("Terminal snapshot exceeds the size limit".into());
    }
    let length = payload.len();
    let payload = match encoding {
        Encoding::Raw => payload,
        Encoding::Deflate => miniz_oxide::deflate::compress_to_vec(&payload, 6),
    };
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&u16::from(matches!(encoding, Encoding::Deflate)).to_le_bytes());
    out.extend_from_slice(&(length as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

pub fn decode(bytes: &[u8]) -> Result<TerminalSnapshot, String> {
    decode_state(bytes).map(|state| state.terminal)
}

pub fn decode_state(bytes: &[u8]) -> Result<NativeState, String> {
    if bytes.len() < HEADER_LEN || !bytes.starts_with(MAGIC) {
        return Err("Invalid terminal snapshot header".into());
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
    let length = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if version != VERSION || flags > 1 || length > MAX_PAYLOAD {
        return Err("Unsupported terminal snapshot header".into());
    }
    let payload = match flags {
        0 if bytes.len() == HEADER_LEN + length => bytes[HEADER_LEN..].to_vec(),
        1 => miniz_oxide::inflate::decompress_to_vec_with_limit(&bytes[HEADER_LEN..], length)
            .map_err(|_| "Invalid compressed terminal snapshot".to_string())?,
        _ => return Err("Invalid terminal snapshot length".into()),
    };
    if payload.len() != length {
        return Err("Invalid terminal snapshot length".into());
    }
    let snapshot: NativeState = serde_json::from_slice(&payload)
        .map_err(|error| format!("Invalid terminal snapshot: {error}"))?;
    snapshot.validate()?;
    Ok(snapshot)
}

pub fn compress(bytes: &[u8]) -> Option<Vec<u8>> {
    let snapshot = decode_state(bytes).ok()?;
    encode_state(&snapshot, Encoding::Deflate).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::{Config, Term, test::TermSize};

    fn snapshot() -> TerminalSnapshot {
        Term::new(Config::default(), &TermSize::new(10, 5), VoidListener).snapshot(100)
    }

    #[test]
    fn raw_compressed_and_background_save_keep_the_same_state() {
        let state = snapshot();
        let raw = encode(&state, Encoding::Raw).unwrap();
        let compressed = encode(&state, Encoding::Deflate).unwrap();
        assert_eq!(decode(&raw).unwrap(), state);
        assert_eq!(decode(&compressed).unwrap(), state);
        let saved = crate::server::binary_snapshot::compress_snapshot(&raw).unwrap();
        assert_eq!(decode(&saved).unwrap(), state);
    }

    #[test]
    fn invalid_lengths_versions_and_payloads_are_rejected() {
        let valid = encode(&snapshot(), Encoding::Raw).unwrap();
        for length in 0..HEADER_LEN {
            assert!(decode(&valid[..length]).is_err());
        }
        let mut invalid = valid.clone();
        invalid[4] = 2;
        assert!(decode(&invalid).is_err());
        let mut invalid = valid.clone();
        invalid[6] = 2;
        assert!(decode(&invalid).is_err());
        let mut invalid = valid.clone();
        invalid[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&invalid).is_err());
        let mut invalid = valid.clone();
        invalid.pop();
        assert!(decode(&invalid).is_err());
        let mut invalid = valid;
        invalid.push(0);
        assert!(decode(&invalid).is_err());
    }

    #[test]
    fn malformed_grid_is_rejected_before_restore() {
        let mut state = snapshot();
        state.main.rows[0].clear();
        assert!(encode(&state, Encoding::Raw).is_err());
        let payload = serde_json::to_vec(&state).unwrap();
        let mut wire = Vec::from(&MAGIC[..]);
        wire.extend_from_slice(&VERSION.to_le_bytes());
        wire.extend_from_slice(&0u16.to_le_bytes());
        wire.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        wire.extend_from_slice(&payload);
        assert!(decode(&wire).is_err());
    }
}
