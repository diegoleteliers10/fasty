//! Version rules for the daemon protocol.
//!
//! Every API change is a [`Rule`] on a JSON pointer (RFC 6901). The first token
//! of the pointer is the message tag (`cmd`, `event`, or `type`). A rule applies
//! to a peer that predates `since`, in both directions: downgrade removes the
//! shape the peer cannot read, upgrade restores the shape this build expects.
//!
//! Rules transform `serde_json::Value` at the connection boundary. Fields that
//! no rule mentions never enter this module.

use serde_json::{Map, Value};

use crate::daemon::{Request, Response, PROTOCOL_VERSION};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    Null,
    Bool(bool),
    Number(i64),
    Text(&'static str),
    EmptyArray,
    EmptyObject,
}

impl Fill {
    fn build(self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(value) => Value::from(value),
            Self::Number(value) => Value::from(value),
            Self::Text(value) => Value::from(value),
            Self::EmptyArray => Value::Array(Vec::new()),
            Self::EmptyObject => Value::Object(Map::new()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The field did not exist before `since`.
    Added,
    /// The field was named `was` before `since`.
    Renamed { was: &'static str },
    /// The field no longer exists from `since` on. Older peers need `fill`.
    Removed { fill: Fill },
}

#[derive(Debug, Clone, Copy)]
pub struct Rule {
    /// Protocol version where the new shape started.
    pub since: u32,
    /// JSON pointer. The first token is the message tag.
    pub path: &'static str,
    pub change: Change,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Downgrade,
    Upgrade,
}

pub const RESPONSE_RULES: &[Rule] = &[
    added(2, "/hello/host_instance"),
    added(2, "/hello/capabilities"),
    added(2, "/attached/program_status"),
    added(2, "/attached/process_running"),
    added(2, "/attached/cwd"),
    added(2, "/attached/shell_pid"),
    added(2, "/attached/mode"),
    added(2, "/session_status/foreground_process_label"),
    added(2, "/close_blocked/foreground_process_label"),
    added(2, "/close_batch/blocked/*/foreground_process_label"),
    added(2, "/sessions/sessions/*/identity"),
    added(2, "/sessions/sessions/*/program_status"),
    added(2, "/session_added/session/identity"),
    added(2, "/session_added/session/program_status"),
    added(2, "/session_updated/session/identity"),
    added(2, "/session_updated/session/program_status"),
    added(2, "/session_updated/terminal_state_changed"),
    added(2, "/event_stream_started/sessions/*/identity"),
    added(2, "/event_stream_started/sessions/*/program_status"),
    added(2, "/event_stream_gap/sessions/*/identity"),
    added(2, "/event_stream_gap/sessions/*/program_status"),
];

pub const REQUEST_RULES: &[Rule] = &[
    added(2, "/attach/expected_host_instance"),
    added(2, "/attach/mode"),
    added(2, "/attach/binary_snapshot"),
    added(2, "/attach/snapshot_format"),
    added(2, "/write/ack"),
    added(2, "/resize/ack"),
    added(2, "/spawn/request_id"),
    added(2, "/subscribe_events/process_events"),
];

const fn added(since: u32, path: &'static str) -> Rule {
    Rule {
        since,
        path,
        change: Change::Added,
    }
}

/// Rewrites a response into the shape a peer at `peer` understands. A `None`
/// peer never announced a version, so it receives the native shape.
pub fn encode_response(response: &Response, peer: Option<u32>) -> Option<String> {
    if matches!(response, Response::Unknown) {
        return None;
    }
    let Some(peer) = peer.filter(|peer| *peer < PROTOCOL_VERSION) else {
        return serde_json::to_string(response).ok();
    };
    if !may_need_transform(response) {
        return serde_json::to_string(response).ok();
    }
    let mut value = serde_json::to_value(response).ok()?;
    apply(&mut value, response.tag(), RESPONSE_RULES, peer, Direction::Downgrade);
    serde_json::to_string(&value).ok()
}

/// Rewrites a request into the shape a peer at `peer` understands.
pub fn encode_request(request: &Request, peer: Option<u32>) -> Result<String, String> {
    let Some(peer) = peer.filter(|peer| *peer < PROTOCOL_VERSION) else {
        return serde_json::to_string(request).map_err(|error| error.to_string());
    };
    if !has_rule(REQUEST_RULES, request.tag()) {
        return serde_json::to_string(request).map_err(|error| error.to_string());
    }
    let mut value = serde_json::to_value(request).map_err(|error| error.to_string())?;
    apply(&mut value, request.tag(), REQUEST_RULES, peer, Direction::Downgrade);
    serde_json::to_string(&value).map_err(|error| error.to_string())
}

/// Reads a request from a peer at `peer`, restoring native shape first.
pub fn parse_request(line: &str, peer: Option<u32>) -> Result<Request, String> {
    let Some(peer) = peer.filter(|peer| *peer < PROTOCOL_VERSION) else {
        return serde_json::from_str(line).map_err(|error| error.to_string());
    };
    let mut value: Value = serde_json::from_str(line).map_err(|error| error.to_string())?;
    let tag = value
        .get("cmd")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    apply(&mut value, &tag, REQUEST_RULES, peer, Direction::Upgrade);
    serde_json::from_value(value).map_err(|error| error.to_string())
}

fn may_need_transform(response: &Response) -> bool {
    match response {
        Response::StreamEvent { .. } => true,
        other => has_rule(RESPONSE_RULES, other.tag()),
    }
}

fn has_rule(rules: &[Rule], tag: &str) -> bool {
    rules
        .iter()
        .filter_map(|rule| split_path(rule.path))
        .any(|(rule_tag, _)| rule_tag == tag)
}

pub fn apply(value: &mut Value, tag: &str, rules: &[Rule], peer: u32, direction: Direction) {
    apply_at(value, Some(tag), rules, peer, direction);
}

fn apply_at(value: &mut Value, tag: Option<&str>, rules: &[Rule], peer: u32, direction: Direction) {
    let tag = match tag {
        Some(tag) => Some(tag.to_owned()),
        None => nested_tag(value).map(str::to_owned),
    };
    if let Some(tag) = tag {
        for rule in rules.iter().filter(|rule| rule.since > peer) {
            let Some((rule_tag, tokens)) = split_path(rule.path) else {
                continue;
            };
            if rule_tag != tag.as_str() {
                continue;
            }
            transform(value, &tokens, rule.change, direction);
        }
    }
    match value {
        Value::Object(map) => {
            for child in map.values_mut() {
                apply_at(child, None, rules, peer, direction);
            }
        }
        Value::Array(items) => {
            for child in items.iter_mut() {
                apply_at(child, None, rules, peer, direction);
            }
        }
        _ => {}
    }
}

fn transform(value: &mut Value, tokens: &[String], change: Change, direction: Direction) {
    let Some((last, parents)) = tokens.split_last() else {
        return;
    };
    let parents: Vec<&str> = parents.iter().map(String::as_str).collect();
    match (change, direction) {
        (Change::Added, Direction::Downgrade) => remove_key(value, &parents, last),
        (Change::Added, Direction::Upgrade) => {}
        (Change::Removed { .. }, Direction::Upgrade) => remove_key(value, &parents, last),
        (Change::Removed { fill }, Direction::Downgrade) => {
            insert_key(value, &parents, last, fill.build())
        }
        (Change::Renamed { was }, Direction::Downgrade) => {
            rename_key(value, &parents, last, was)
        }
        (Change::Renamed { was }, Direction::Upgrade) => {
            rename_key(value, &parents, was, last)
        }
    }
}

fn remove_key(value: &mut Value, parents: &[&str], key: &str) {
    for_each_node(value, parents, &mut |parent| {
        if let Value::Object(map) = parent {
            map.remove(key);
        }
    });
}

fn insert_key(value: &mut Value, parents: &[&str], key: &str, fill: Value) {
    for_each_node(value, parents, &mut |parent| {
        if let Value::Object(map) = parent {
            map.entry(key.to_string()).or_insert_with(|| fill.clone());
        }
    });
}

fn rename_key(value: &mut Value, parents: &[&str], from: &str, to: &str) {
    if from == to {
        return;
    }
    for_each_node(value, parents, &mut |parent| {
        if let Value::Object(map) = parent {
            if let Some(field) = map.remove(from) {
                map.insert(to.to_string(), field);
            }
        }
    });
}

fn for_each_node<F: FnMut(&mut Value)>(value: &mut Value, tokens: &[&str], op: &mut F) {
    let Some((token, rest)) = tokens.split_first() else {
        op(value);
        return;
    };
    match value {
        Value::Object(map) if *token == "*" => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if let Some(child) = map.get_mut(&key) {
                    for_each_node(child, rest, op);
                }
            }
        }
        Value::Object(map) => {
            if let Some(child) = map.get_mut(*token) {
                for_each_node(child, rest, op);
            }
        }
        Value::Array(items) if *token == "*" => {
            for child in items.iter_mut() {
                for_each_node(child, rest, op);
            }
        }
        Value::Array(items) => {
            if let Ok(index) = token.parse::<usize>() {
                if let Some(child) = items.get_mut(index) {
                    for_each_node(child, rest, op);
                }
            }
        }
        _ => {}
    }
}

/// A tagged union nested inside a message carries `type`. The root message
/// tag comes from the caller, never from the payload.
fn nested_tag(value: &Value) -> Option<&str> {
    value.as_object()?.get("type")?.as_str()
}

fn split_path(path: &str) -> Option<(&str, Vec<String>)> {
    let mut tokens = path.strip_prefix('/')?.split('/');
    let tag = tokens.next()?;
    Some((tag, tokens.map(unescape).collect()))
}

fn unescape(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const RENAME: &[Rule] = &[Rule {
        since: 3,
        path: "/attached/working_directory",
        change: Change::Renamed { was: "cwd" },
    }];

    const REMOVAL: &[Rule] = &[Rule {
        since: 3,
        path: "/attached/cols",
        change: Change::Removed {
            fill: Fill::Number(80),
        },
    }];

    fn apply_message(value: &mut Value, rules: &[Rule], peer: u32, direction: Direction) {
        let tag = value
            .get("event")
            .or_else(|| value.get("cmd"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        apply(value, &tag, rules, peer, direction);
    }

    #[test]
    fn added_rule_drops_the_field_for_older_peers() {
        let mut value = json!({"event": "hello", "version": 2, "capabilities": ["a"]});
        apply_message(&mut value, RESPONSE_RULES, 1, Direction::Downgrade);
        assert!(value.get("capabilities").is_none());
        assert_eq!(value["version"], 2);
    }

    #[test]
    fn added_rule_leaves_matching_peers_alone() {
        let mut value = json!({"event": "hello", "capabilities": ["a"]});
        apply_message(&mut value, RESPONSE_RULES, 2, Direction::Downgrade);
        assert_eq!(value["capabilities"], json!(["a"]));
    }

    #[test]
    fn removed_rule_restores_the_field_for_older_peers() {
        let mut value = json!({"event": "attached", "id": 1});
        apply_message(&mut value, REMOVAL, 2, Direction::Downgrade);
        assert_eq!(value["cols"], 80);
    }

    #[test]
    fn removed_rule_keeps_an_existing_value() {
        let mut value = json!({"event": "attached", "cols": 120});
        apply_message(&mut value, REMOVAL, 2, Direction::Downgrade);
        assert_eq!(value["cols"], 120);
    }

    #[test]
    fn removed_rule_drops_the_field_when_upgrading() {
        let mut value = json!({"event": "attached", "cols": 80});
        apply_message(&mut value, REMOVAL, 2, Direction::Upgrade);
        assert!(value.get("cols").is_none());
    }

    #[test]
    fn renamed_rule_moves_the_key_in_both_directions() {
        let mut downgraded = json!({"event": "attached", "working_directory": "/tmp"});
        apply_message(&mut downgraded, RENAME, 2, Direction::Downgrade);
        assert_eq!(downgraded["cwd"], "/tmp");
        assert!(downgraded.get("working_directory").is_none());

        let mut upgraded = json!({"event": "attached", "cwd": "/tmp"});
        apply_message(&mut upgraded, RENAME, 2, Direction::Upgrade);
        assert_eq!(upgraded["working_directory"], "/tmp");
        assert!(upgraded.get("cwd").is_none());
    }

    #[test]
    fn wildcard_path_reaches_every_array_item() {
        let mut value = json!({
            "event": "sessions",
            "sessions": [
                {"id": 1, "program_status": [], "title": "a"},
                {"id": 2, "program_status": [], "title": "b"}
            ]
        });
        apply_message(&mut value, RESPONSE_RULES, 1, Direction::Downgrade);
        for session in value["sessions"].as_array().unwrap() {
            assert!(session.get("program_status").is_none());
            assert!(session.get("title").is_some());
        }
    }

    #[test]
    fn rules_ignore_other_message_tags() {
        let mut value = json!({"event": "sessions", "capabilities": ["a"]});
        apply_message(&mut value, RESPONSE_RULES, 1, Direction::Downgrade);
        assert_eq!(value["capabilities"], json!(["a"]));
    }

    #[test]
    fn nested_tagged_messages_are_transformed() {
        let mut value = json!({
            "event": "stream_event",
            "envelope": {
                "version": 1,
                "sequence": 7,
                "timestamp_unix_ms": 0,
                "session_id": 1,
                "kind": {
                    "type": "session_updated",
                    "session": {"id": 1, "program_status": []},
                    "terminal_state_changed": true
                }
            }
        });
        apply_message(&mut value, RESPONSE_RULES, 1, Direction::Downgrade);
        assert!(value["envelope"]["kind"].get("terminal_state_changed").is_none());
        assert!(value["envelope"]["kind"]["session"].get("program_status").is_none());
    }

    #[test]
    fn pointer_tokens_follow_rfc_6901_escaping() {
        let (tag, tokens) = split_path("/attached/a~1b/c~0d").unwrap();
        assert_eq!(tag, "attached");
        assert_eq!(tokens, vec!["a/b".to_string(), "c~d".to_string()]);
    }

    #[test]
    fn native_peer_encoding_is_unchanged() {
        let response = Response::Done {
            cmd: "write".to_string(),
        };
        let native = serde_json::to_string(&response).unwrap();
        assert_eq!(encode_response(&response, None).unwrap(), native);
        assert_eq!(
            encode_response(&response, Some(PROTOCOL_VERSION)).unwrap(),
            native
        );
    }

    #[test]
    fn unknown_response_is_never_encoded() {
        assert!(encode_response(&Response::Unknown, None).is_none());
        assert!(encode_response(&Response::Unknown, Some(1)).is_none());
    }

    #[test]
    fn unknown_event_parses_instead_of_failing() {
        let response: Response =
            serde_json::from_str(r#"{"event":"future_command","payload":{"any":1}}"#).unwrap();
        assert!(matches!(response, Response::Unknown));

        let kind: crate::daemon::SessionEvent =
            serde_json::from_str(r#"{"type":"future_session_event"}"#).unwrap();
        assert!(matches!(kind, crate::daemon::SessionEvent::Unknown));
    }

    #[test]
    fn unknown_request_command_still_fails() {
        assert!(parse_request(r#"{"cmd":"future_command"}"#, None).is_err());
    }

    fn minimal(schema: &Value, defs: &Value) -> Option<Value> {
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            let name = reference.rsplit('/').next()?;
            return minimal(defs.get(name)?, defs);
        }
        if let Some(value) = schema.get("const") {
            return Some(value.clone());
        }
        if let Some(value) = schema.get("enum").and_then(Value::as_array).and_then(|values| values.first()) {
            return Some(value.clone());
        }
        for key in ["oneOf", "anyOf"] {
            if let Some(list) = schema.get(key).and_then(Value::as_array) {
                if let Some(value) = list.iter().find_map(|variant| minimal(variant, defs)) {
                    return Some(value);
                }
            }
        }
        if let Some(first) = schema.get("allOf").and_then(Value::as_array).and_then(|list| list.first()) {
            return minimal(first, defs);
        }
        let kind = match schema.get("type") {
            Some(Value::String(kind)) => kind.as_str(),
            Some(Value::Array(kinds)) => kinds
                .iter()
                .filter_map(Value::as_str)
                .find(|kind| *kind != "null")?,
            _ => "object",
        };
        match kind {
            "object" => {
                let required = schema
                    .get("required")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let properties = schema.get("properties");
                let mut map = Map::new();
                for name in required.iter().filter_map(Value::as_str) {
                    let property = properties.and_then(|properties| properties.get(name))?;
                    map.insert(name.to_string(), minimal(property, defs)?);
                }
                Some(Value::Object(map))
            }
            "array" => Some(Value::Array(Vec::new())),
            "string" => Some(Value::from("")),
            "integer" | "number" => Some(Value::from(0)),
            "boolean" => Some(Value::from(false)),
            "null" => Some(Value::Null),
            _ => None,
        }
    }

    fn variants<T: schemars::JsonSchema>(tag: &str) -> Vec<(String, Value)> {
        let document = serde_json::to_value(schemars::schema_for!(T)).unwrap();
        let definitions = document
            .get("definitions")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let list = document["oneOf"]
            .as_array()
            .unwrap_or_else(|| panic!("the schema has no variants"));
        list.iter()
            .map(|variant| {
                let name = variant["properties"][tag]["enum"][0]
                    .as_str()
                    .unwrap_or_else(|| panic!("a variant has no {tag} name"))
                    .to_string();
                let instance = minimal(variant, &definitions)
                    .unwrap_or_else(|| panic!("cannot build a minimal {name} value"));
                (name, instance)
            })
            .collect()
    }

    #[test]
    fn every_rule_names_a_real_field() {
        let responses = serde_json::to_value(schemars::schema_for!(Response)).unwrap();
        let events =
            serde_json::to_value(schemars::schema_for!(crate::daemon::SessionEvent)).unwrap();
        let requests = serde_json::to_value(schemars::schema_for!(Request)).unwrap();
        for rule in RESPONSE_RULES {
            let (tag, tokens) = split_path(rule.path).unwrap();
            assert!(
                schema_has_leaf(&responses, "event", tag, &tokens)
                    || schema_has_leaf(&events, "type", tag, &tokens),
                "{} names no field in any response",
                rule.path
            );
        }
        for rule in REQUEST_RULES {
            let (tag, tokens) = split_path(rule.path).unwrap();
            assert!(
                schema_has_leaf(&requests, "cmd", tag, &tokens),
                "{} names no field in any request",
                rule.path
            );
        }
    }

    #[test]
    fn every_rule_removes_the_field_it_names() {
        let mut responses = wire_instances::<Response>("event");
        responses.extend(wire_instances::<crate::daemon::SessionEvent>("type"));
        for rule in RESPONSE_RULES {
            assert_leaf_removed(rule, RESPONSE_RULES, &responses);
        }
        let requests = wire_instances::<Request>("cmd");
        for rule in REQUEST_RULES {
            assert_leaf_removed(rule, REQUEST_RULES, &requests);
        }
    }

    fn assert_leaf_removed(rule: &Rule, table: &[Rule], instances: &[(String, Value)]) {
        let (tag, tokens) = split_path(rule.path).unwrap();
        let mut seen = false;
        for (name, instance) in instances.iter().filter(|(name, _)| name == tag) {
            let mut value = instance.clone();
            if !has_leaf(&mut value, &tokens) {
                continue;
            }
            seen = true;
            apply(&mut value, tag, table, 1, Direction::Downgrade);
            assert!(
                !has_leaf(&mut value, &tokens),
                "{} did not remove its field from {name}",
                rule.path
            );
        }
        assert!(seen, "{} matches no emitted payload", rule.path);
    }

    fn schema_has_leaf(document: &Value, tag_key: &str, tag: &str, tokens: &[String]) -> bool {
        let defs = document
            .get("definitions")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let Some(variants) = document["oneOf"].as_array() else {
            return false;
        };
        let Some(variant) = variants
            .iter()
            .find(|variant| variant["properties"][tag_key]["enum"][0].as_str() == Some(tag))
        else {
            return false;
        };
        let mut schema = resolve(variant, &defs);
        for token in tokens {
            schema = resolve(&schema, &defs);
            let Some(next) = (if token == "*" {
                schema.get("items")
            } else {
                schema.get("properties").and_then(|properties| properties.get(token))
            }) else {
                return false;
            };
            schema = next.clone();
        }
        true
    }

    fn resolve(schema: &Value, defs: &Value) -> Value {
        let mut current = schema.clone();
        for _ in 0..8 {
            if let Some(reference) = current.get("$ref").and_then(Value::as_str) {
                let name = reference.rsplit('/').next().unwrap_or_default();
                match defs.get(name) {
                    Some(resolved) => {
                        current = resolved.clone();
                        continue;
                    }
                    None => break,
                }
            }
            let union = current
                .get("anyOf")
                .or_else(|| current.get("oneOf"))
                .and_then(Value::as_array);
            if let Some(list) = union {
                let pick = list
                    .iter()
                    .find(|option| option.get("type").and_then(Value::as_str) != Some("null"));
                if let Some(pick) = pick {
                    current = pick.clone();
                    continue;
                }
            }
            break;
        }
        current
    }

    fn wire_instances<T>(tag: &str) -> Vec<(String, Value)>
    where
        T: schemars::JsonSchema + serde::Serialize + serde::de::DeserializeOwned,
    {
        let document = serde_json::to_value(schemars::schema_for!(T)).unwrap();
        let definitions = document
            .get("definitions")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let list = document["oneOf"]
            .as_array()
            .unwrap_or_else(|| panic!("the schema has no variants"));
        list.iter()
            .map(|variant| {
                let name = variant["properties"][tag]["enum"][0]
                    .as_str()
                    .unwrap_or_else(|| panic!("a variant has no {tag} name"))
                    .to_string();
                let instance = maximal(variant, &definitions, 0)
                    .unwrap_or_else(|| panic!("cannot build a {name} value"));
                let typed: T = serde_json::from_value(instance)
                    .unwrap_or_else(|error| panic!("{name} does not parse: {error}"));
                let wire = serde_json::to_value(&typed).unwrap();
                (name, wire)
            })
            .collect()
    }

    fn maximal(schema: &Value, defs: &Value, depth: usize) -> Option<Value> {
        if depth > 24 {
            return None;
        }
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            let name = reference.rsplit('/').next()?;
            return maximal(defs.get(name)?, defs, depth + 1);
        }
        if let Some(value) = schema.get("const") {
            return Some(value.clone());
        }
        if let Some(value) = schema
            .get("enum")
            .and_then(Value::as_array)
            .and_then(|values| values.last())
        {
            return Some(value.clone());
        }
        for key in ["oneOf", "anyOf"] {
            if let Some(list) = schema.get(key).and_then(Value::as_array) {
                if let Some(value) = list
                    .iter()
                    .find_map(|variant| maximal(variant, defs, depth + 1))
                {
                    return Some(value);
                }
            }
        }
        if let Some(first) = schema
            .get("allOf")
            .and_then(Value::as_array)
            .and_then(|list| list.first())
        {
            return maximal(first, defs, depth + 1);
        }
        let kind = match schema.get("type") {
            Some(Value::String(kind)) => kind.as_str(),
            Some(Value::Array(kinds)) => kinds
                .iter()
                .filter_map(Value::as_str)
                .find(|kind| *kind != "null")?,
            _ => "object",
        };
        match kind {
            "object" => {
                let mut map = Map::new();
                if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                    for (name, property) in properties {
                        if let Some(value) = maximal(property, defs, depth + 1) {
                            map.insert(name.clone(), value);
                        }
                    }
                }
                Some(Value::Object(map))
            }
            "array" => match schema.get("items") {
                Some(items) => Some(Value::Array(vec![maximal(items, defs, depth + 1)?])),
                None => Some(Value::Array(Vec::new())),
            },
            "string" => Some(Value::from("")),
            "integer" | "number" => Some(Value::from(0)),
            "boolean" => Some(Value::from(true)),
            "null" => Some(Value::Null),
            _ => None,
        }
    }

    fn has_leaf(value: &mut Value, tokens: &[String]) -> bool {
        let Some((key, parents)) = tokens.split_last() else {
            return false;
        };
        let parents: Vec<&str> = parents.iter().map(String::as_str).collect();
        let mut found = false;
        for_each_node(value, &parents, &mut |parent| {
            if let Value::Object(map) = parent {
                found |= map.contains_key(key);
            }
        });
        found
    }

    #[test]
    fn every_response_parses_after_a_downgrade_to_every_older_version() {
        let variants = variants::<Response>("event");
        assert!(variants.len() >= 30, "only {} variants", variants.len());
        for (name, instance) in variants {
            let native: Response = serde_json::from_value(instance.clone())
                .unwrap_or_else(|error| panic!("{name} does not parse natively: {error}"));
            assert_eq!(native.tag(), name);
            for peer in 1..PROTOCOL_VERSION {
                let mut value = instance.clone();
                apply_message(&mut value, RESPONSE_RULES, peer, Direction::Downgrade);
                let parsed: Response = serde_json::from_value(value.clone())
                    .unwrap_or_else(|error| panic!("{name} does not parse at {peer}: {error}"));
                assert_eq!(parsed.tag(), name, "{name} changed tag at {peer}");
                assert!(
                    !matches!(parsed, Response::Unknown),
                    "{name} became unknown at {peer}"
                );
            }
        }
    }

    #[test]
    fn every_request_parses_after_an_upgrade_to_native_shape() {
        let variants = variants::<Request>("cmd");
        assert!(variants.len() >= 28, "only {} variants", variants.len());
        for (name, instance) in variants {
            let native: Request = serde_json::from_value(instance.clone())
                .unwrap_or_else(|error| panic!("{name} does not parse natively: {error}"));
            assert_eq!(native.tag(), name);
            for peer in 1..PROTOCOL_VERSION {
                let mut older = instance.clone();
                apply_message(&mut older, REQUEST_RULES, peer, Direction::Downgrade);
                let parsed: Request = serde_json::from_value(older.clone())
                    .unwrap_or_else(|error| panic!("{name} does not parse at {peer}: {error}"));
                assert_eq!(parsed.tag(), name, "{name} changed tag at {peer}");

                let mut upgraded = older;
                apply_message(&mut upgraded, REQUEST_RULES, peer, Direction::Upgrade);
                serde_json::from_value::<Request>(upgraded)
                    .unwrap_or_else(|error| panic!("{name} does not upgrade at {peer}: {error}"));
            }
        }
    }

    #[test]
    fn schema_never_advertises_unknown_variants() {
        let response = serde_json::to_value(schemars::schema_for!(Response)).unwrap();
        let response_tags: Vec<&str> = response["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|variant| variant["properties"]["event"]["enum"][0].as_str())
            .collect();
        assert!(!response_tags.contains(&"unknown"));

        let request = serde_json::to_value(schemars::schema_for!(Request)).unwrap();
        let request_tags: Vec<&str> = request["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|variant| variant["properties"]["cmd"]["enum"][0].as_str())
            .collect();
        assert!(!request_tags.contains(&"unknown"));
    }

    #[test]
    fn downgraded_attach_requests_drop_every_v2_field() {
        let request = Request::Attach {
            id: 1,
            expected_host_instance: Some(crate::daemon::HostInstanceId("host".into())),
            mode: crate::daemon::AttachMode::ReadOnly,
            binary_snapshot: true,
            snapshot_format: crate::daemon::SnapshotFormat::TerminalState,
        };
        let native = serde_json::to_string(&request).unwrap();
        assert_eq!(encode_request(&request, None).unwrap(), native);
        assert_eq!(encode_request(&request, Some(PROTOCOL_VERSION)).unwrap(), native);

        let old: Value = serde_json::from_str(&encode_request(&request, Some(1)).unwrap()).unwrap();
        assert_eq!(old["cmd"], "attach");
        assert_eq!(old["id"], 1);
        assert!(old.get("mode").is_none());
        assert!(old.get("binary_snapshot").is_none());
        assert!(old.get("snapshot_format").is_none());
        assert!(old.get("expected_host_instance").is_none());
    }

    #[test]
    fn a_v1_attached_payload_parses_into_the_native_shape() {
        let parsed: Response =
            serde_json::from_value(json!({"event": "attached", "id": 1, "cols": 80, "rows": 24}))
                .unwrap();
        match parsed {
            Response::Attached { cols, mode, .. } => {
                assert_eq!(cols, 80);
                assert_eq!(mode, crate::daemon::AttachMode::ReadWrite);
            }
            other => panic!("unexpected variant: {}", other.tag()),
        }
    }

    #[test]
    fn downgraded_attached_responses_drop_every_v2_field() {
        let mut value = serde_json::to_value(Response::Attached {
            id: 1,
            cols: 80,
            rows: 24,
            mode: crate::daemon::AttachMode::ReadOnly,
            program_status: Vec::new(),
            process_running: Some(true),
            cwd: Some("/tmp".into()),
            shell_pid: Some(42),
        })
        .unwrap();
        apply_message(&mut value, RESPONSE_RULES, 1, Direction::Downgrade);
        assert!(value.get("program_status").is_none());
        assert!(value.get("process_running").is_none());
        assert!(value.get("cwd").is_none());
        assert!(value.get("shell_pid").is_none());
        assert!(value.get("mode").is_none());
        assert_eq!(value["cols"], 80);
    }

    #[test]
    fn downgraded_responses_drop_every_v2_field() {
        let mut value = serde_json::to_value(Response::Hello {
            host_instance: Some(crate::daemon::HostInstanceId("host".into())),
            version: PROTOCOL_VERSION,
            fastty_version: "0.0.0".into(),
            capabilities: vec!["cap".into()],
        })
        .unwrap();
        apply_message(&mut value, RESPONSE_RULES, 1, Direction::Downgrade);
        assert!(value.get("capabilities").is_none());
        assert!(value.get("host_instance").is_none());
        assert_eq!(value["version"], PROTOCOL_VERSION);
    }
}
