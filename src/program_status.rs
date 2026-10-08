//! Program Status Protocol records for one terminal.

use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::DecodePaddingMode;
use base64::Engine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramState {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}

impl ProgramState {
    pub fn priority(self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Done => 1,
            Self::Working => 2,
            Self::Error => 3,
            Self::Blocked => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockedKind {
    Permission,
    Question,
    Auth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramRecord {
    pub id: String,
    pub state: ProgramState,
    pub app: Option<String>,
    pub title: Option<String>,
    pub msg: Option<String>,
    pub progress: Option<u8>,
    pub kind: Option<BlockedKind>,
}

pub fn aggregate(records: &[ProgramRecord]) -> Option<&ProgramRecord> {
    records.iter().max_by_key(|record| record.state.priority())
}

#[derive(Debug, Clone)]
pub(crate) enum ProgramStatusCommand {
    Query,
    Clear(String),
    Report(ProgramRecord),
}

#[derive(Default)]
pub(crate) struct ProgramStatusStore {
    records: Vec<ProgramRecord>,
}

impl ProgramStatusStore {
    pub(crate) fn apply(&mut self, command: ProgramStatusCommand) {
        match command {
            ProgramStatusCommand::Query => {}
            ProgramStatusCommand::Clear(id) => self.records.retain(|record| {
                !id.is_empty()
                    && record.id != id
                    && !record
                        .id
                        .strip_prefix(&id)
                        .is_some_and(|tail| tail.starts_with('/'))
            }),
            ProgramStatusCommand::Report(record) => {
                self.records.retain(|stored| stored.id != record.id);
                if self.records.len() == 256 {
                    self.records.remove(0);
                }
                self.records.push(record);
            }
        }
    }

    pub(crate) fn end_activity(&mut self) {
        self.records.retain(|record| {
            !matches!(record.state, ProgramState::Working | ProgramState::Blocked)
        });
    }

    pub(crate) fn reset(&mut self) {
        self.records.clear();
    }

    pub(crate) fn snapshot(&self) -> Vec<ProgramRecord> {
        self.records
            .iter()
            .map(|record| {
                let mut resolved = record.clone();
                let mut id = record.id.as_str();
                while resolved.app.is_none() && !id.is_empty() {
                    id = id.rsplit_once('/').map_or("", |(parent, _)| parent);
                    resolved.app = self
                        .records
                        .iter()
                        .find(|ancestor| ancestor.id == id)
                        .and_then(|ancestor| ancestor.app.clone());
                }
                resolved
            })
            .collect()
    }
}

pub(crate) fn parse(payload: &[u8], sequence_len: usize) -> Option<ProgramStatusCommand> {
    if sequence_len > 4096 {
        return None;
    }
    if payload == b"?" {
        return Some(ProgramStatusCommand::Query);
    }
    let mut fields = std::collections::HashMap::new();
    for pair in payload.split(|byte| *byte == b':') {
        let Some(separator) = pair.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let key = trim_ascii(&pair[..separator]);
        let value = trim_ascii(&pair[separator + 1..]);
        if key.len() > 16 {
            return None;
        }
        match key {
            b"app" if value.len() > 32 => return None,
            b"msg" if value.len() > 2732 => return None,
            b"title" if value.len() > 256 => return None,
            b"id"
                if value.len() > 128
                    || value.split(|byte| *byte == b'/').count() > 8
                    || value
                        .split(|byte| *byte == b'/')
                        .any(|segment| segment.len() > 32) =>
            {
                return None
            }
            _ => {}
        }
        if key == b"id" && !valid_id(value) {
            fields.insert(key, value);
            continue;
        }
        if key.is_empty()
            || !key.iter().all(u8::is_ascii_lowercase)
            || !value
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.,+/=-".contains(byte))
        {
            continue;
        }
        match key {
            b"msg" => {
                decode_text(value, 2732, 2048)?;
            }
            b"title" => {
                decode_text(value, 256, 192)?;
            }
            _ => {}
        }
        fields.insert(key, value);
    }
    let id = match fields.get(b"id".as_slice()) {
        Some(value) if valid_id(value) => std::str::from_utf8(value).ok()?.to_owned(),
        Some(_) => return None,
        None => String::new(),
    };
    let state = match *fields.get(b"state".as_slice())? {
        b"idle" => ProgramState::Idle,
        b"working" => ProgramState::Working,
        b"done" => ProgramState::Done,
        b"blocked" => ProgramState::Blocked,
        b"error" => ProgramState::Error,
        b"clear" => return Some(ProgramStatusCommand::Clear(id)),
        _ => return None,
    };
    let app = fields
        .get(b"app".as_slice())
        .filter(|value| valid_segment(value))
        .and_then(|value| std::str::from_utf8(value).ok())
        .map(str::to_owned);
    let kind = if state == ProgramState::Blocked {
        match fields.get(b"kind".as_slice()).copied() {
            Some(b"permission") => Some(BlockedKind::Permission),
            Some(b"question") => Some(BlockedKind::Question),
            Some(b"auth") => Some(BlockedKind::Auth),
            _ => None,
        }
    } else {
        None
    };
    let progress = if matches!(state, ProgramState::Working | ProgramState::Blocked) {
        fields
            .get(b"progress".as_slice())
            .filter(|value| !value.is_empty() && value.iter().all(u8::is_ascii_digit))
            .and_then(|value| std::str::from_utf8(value).ok())
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|value| *value <= 100)
    } else {
        None
    };
    let title = match fields.get(b"title".as_slice()) {
        Some(value) => Some(decode_text(value, 256, 192)?),
        None => None,
    };
    let msg = match fields.get(b"msg".as_slice()) {
        Some(value) => Some(decode_text(value, 2732, 2048)?),
        None => None,
    };
    Some(ProgramStatusCommand::Report(ProgramRecord {
        id,
        state,
        app,
        title,
        msg,
        progress,
        kind,
    }))
}

fn trim_ascii(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    &value[start..end]
}

fn valid_segment(value: &[u8]) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.+-".contains(byte))
}

fn valid_id(value: &[u8]) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.split(|byte| *byte == b'/').count() <= 8
        && value.split(|byte| *byte == b'/').all(valid_segment)
}

fn decode_text(value: &[u8], encoded_limit: usize, decoded_limit: usize) -> Option<String> {
    if value.len() > encoded_limit {
        return None;
    }
    let decoder = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    let bytes = decoder.decode(value).ok()?;
    if bytes.len() > decoded_limit {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    if text
        .chars()
        .any(|character| matches!(character as u32, 0..=31 | 127..=159))
    {
        return None;
    }
    Some(
        text.chars()
            .filter(|character| {
                !matches!(*character as u32,
        0x00ad | 0x034f | 0x061c | 0x115f..=0x1160 | 0x17b4..=0x17b5 |
        0x180b..=0x180f | 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x206f |
        0x3164 | 0xfeff | 0xffa0 | 0xfff9..=0xfffb | 0x1bca0..=0x1bca3 |
        0x1d173..=0x1d17a | 0xe0000..=0xe007f)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    fn command(body: &str) -> ProgramStatusCommand {
        parse(body.as_bytes(), body.len() + 9).expect("valid protocol report")
    }

    fn report(store: &mut ProgramStatusStore, body: &str) {
        store.apply(command(body));
    }

    fn record<'a>(records: &'a [ProgramRecord], id: &str) -> &'a ProgramRecord {
        records
            .iter()
            .find(|record| record.id == id)
            .expect("record exists")
    }

    #[test]
    fn invalid_id_does_not_replace_root() {
        let mut store = ProgramStatusStore::default();
        report(&mut store, "state=done:app=cargo");
        let before = store.snapshot();
        for id in ["", "/child", "parent/", "parent//child", "bad id", "bad;id"] {
            let body = format!("state=error:id={id}");
            assert!(parse(body.as_bytes(), body.len() + 9).is_none(), "{id}");
        }
        assert_eq!(store.snapshot(), before);
    }

    #[test]
    fn invalid_text_discards_the_whole_report() {
        let mut store = ProgramStatusStore::default();
        report(&mut store, "state=done:app=cargo");
        let before = store.snapshot();
        for field in ["msg", "title"] {
            for encoded in ["A", "!!!!", "AB==", "/w=="] {
                let body = format!("state=error:app=other:{field}={encoded}");
                let parsed = parse(body.as_bytes(), body.len() + 9);
                if encoded == "!!!!" {
                    // The value alphabet rule skips this malformed pair.
                    assert!(parsed.is_some());
                } else {
                    assert!(parsed.is_none(), "{field}={encoded}");
                }
            }
            for text in ["a\nb", "\u{7f}", "\u{80}", "\u{9f}"] {
                let encoded = STANDARD.encode(text);
                let body = format!("state=error:app=other:{field}={encoded}");
                assert!(parse(body.as_bytes(), body.len() + 9).is_none());
            }
            let body = format!("state=error:{field}=A:{field}=dmFsaWQ=");
            assert!(parse(body.as_bytes(), body.len() + 9).is_none());
        }
        assert_eq!(store.snapshot(), before);
    }

    #[test]
    fn last_duplicate_key_wins() {
        let mut store = ProgramStatusStore::default();
        report(&mut store,
            "state=idle:state=blocked:id=bad id:id=job:app=one:app=two:kind=auth:kind=question:progress=1:progress=75:msg=b25l:msg=dHdv");
        let records = store.snapshot();
        let job = record(&records, "job");
        assert_eq!(job.state, ProgramState::Blocked);
        assert_eq!(job.app.as_deref(), Some("two"));
        assert_eq!(job.kind, Some(BlockedKind::Question));
        assert_eq!(job.progress, Some(75));
        assert_eq!(job.msg.as_deref(), Some("two"));
    }

    #[test]
    fn report_limits_are_hard_caps() {
        assert!(parse(b"state=working", 4096).is_some());
        assert!(parse(b"state=working", 4097).is_none());
        for body in [
            format!("state=working:{}=x", "k".repeat(17)),
            format!("state=working:app={}", "a".repeat(33)),
            format!("state=working:id={}", "a".repeat(33)),
            format!("state=working:id={}", ["a"; 9].join("/")),
            format!("state=working:id={}", vec!["a".repeat(32); 4].join("/")),
            format!("state=working:msg={}", "A".repeat(2733)),
            format!("state=working:title={}", "A".repeat(257)),
            format!("state=working:msg={}", STANDARD.encode(vec![b'a'; 2049])),
        ] {
            assert!(parse(body.as_bytes(), body.len() + 9).is_none());
        }
        let body = format!("state=working:msg={}", STANDARD.encode(vec![b'a'; 2048]));
        assert!(parse(body.as_bytes(), body.len() + 9).is_some());
        let body = format!("state=working:title={}", STANDARD.encode(vec![b'a'; 192]));
        assert!(parse(body.as_bytes(), body.len() + 9).is_some());
    }

    #[test]
    fn reports_replace_all_optional_fields() {
        let mut store = ProgramStatusStore::default();
        report(
            &mut store,
            "state=blocked:id=job:app=cargo:kind=permission:progress=50:title=Sm9i:msg=V2FpdA==",
        );
        report(&mut store, "state=working:id=job");
        assert_eq!(
            store.snapshot(),
            vec![ProgramRecord {
                id: "job".to_owned(),
                state: ProgramState::Working,
                app: None,
                kind: None,
                progress: None,
                title: None,
                msg: None,
            }]
        );
    }

    #[test]
    fn descendants_use_the_nearest_current_ancestor_app() {
        let mut store = ProgramStatusStore::default();
        report(&mut store, "state=working:app=root");
        report(&mut store, "state=working:id=job/child");
        assert_eq!(
            record(&store.snapshot(), "job/child").app.as_deref(),
            Some("root")
        );
        report(&mut store, "state=working:id=job:app=parent");
        assert_eq!(
            record(&store.snapshot(), "job/child").app.as_deref(),
            Some("parent")
        );
        report(&mut store, "state=working:id=job");
        assert_eq!(
            record(&store.snapshot(), "job/child").app.as_deref(),
            Some("root")
        );
        report(&mut store, "state=done:app=new-root");
        assert_eq!(
            record(&store.snapshot(), "job/child").app.as_deref(),
            Some("new-root")
        );
    }

    #[test]
    fn clear_removes_only_the_addressed_subtree() {
        let mut store = ProgramStatusStore::default();
        for id in ["build", "build/test", "builder", "build-two"] {
            report(&mut store, &format!("state=done:id={id}"));
        }
        report(&mut store, "state=clear:id=build");
        let records = store.snapshot();
        assert_eq!(records.len(), 2);
        assert_eq!(record(&records, "builder").state, ProgramState::Done);
        assert_eq!(record(&records, "build-two").state, ProgramState::Done);
        report(&mut store, "state=clear");
        assert!(store.snapshot().is_empty());
    }

    #[test]
    fn record_cap_evicts_the_least_recently_updated_record() {
        let mut store = ProgramStatusStore::default();
        for index in 0..256 {
            report(&mut store, &format!("state=working:id=job{index}"));
        }
        report(&mut store, "state=done:id=job0");
        report(&mut store, "state=working:id=new");
        let records = store.snapshot();
        assert_eq!(records.len(), 256);
        assert_eq!(record(&records, "job0").state, ProgramState::Done);
        assert!(!records.iter().any(|record| record.id == "job1"));
        assert_eq!(record(&records, "new").state, ProgramState::Working);
    }

    #[test]
    fn activity_end_preserves_completed_records_and_reset_removes_all() {
        let mut store = ProgramStatusStore::default();
        for state in ["idle", "working", "blocked", "done", "error"] {
            report(&mut store, &format!("state={state}:id={state}"));
        }
        store.end_activity();
        let records = store.snapshot();
        assert_eq!(records.len(), 3);
        assert_eq!(record(&records, "done").state, ProgramState::Done);
        assert_eq!(record(&records, "error").state, ProgramState::Error);
        assert_eq!(record(&records, "idle").state, ProgramState::Idle);
        store.reset();
        assert!(store.snapshot().is_empty());
    }
}
