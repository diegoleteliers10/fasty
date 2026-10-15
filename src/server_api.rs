use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApiServer {
    pub schema_version: u32,
    pub server_version: String,
    pub protocol_version: u32,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApiMethod {
    pub name: String,
    pub description: String,
    pub response_events: Vec<String>,
    pub response_mode: ApiResponseMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiResponseMode {
    Single,
    Stream,
    AckOptional,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApiCatalog {
    #[serde(flatten)]
    pub server: ApiServer,
    pub methods: Vec<ApiMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApiMethodDescription {
    #[serde(flatten)]
    pub server: ApiServer,
    pub method: ApiMethod,
    pub request_schema: Value,
    pub response_schema: Value,
}

fn server() -> ApiServer {
    ApiServer {
        schema_version: 1,
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: crate::daemon::PROTOCOL_VERSION,
        capabilities: crate::daemon::session_host_capabilities(),
    }
}

fn schema<T: JsonSchema>() -> Result<Value, String> {
    serde_json::to_value(schemars::schema_for!(T)).map_err(|error| error.to_string())
}

fn tagged_variants(document: &Value, tag: &str) -> Result<BTreeMap<String, Value>, String> {
    let variants = document["oneOf"]
        .as_array()
        .ok_or_else(|| format!("the {tag} schema has no variants"))?;
    variants
        .iter()
        .map(|variant| {
            let name = variant["properties"][tag]["enum"][0]
                .as_str()
                .ok_or_else(|| format!("a schema variant has no {tag} name"))?;
            Ok((name.to_string(), variant.clone()))
        })
        .collect()
}

fn method(name: &str, request_schema: &Value) -> Result<ApiMethod, String> {
    use ApiResponseMode::{AckOptional, Single, Stream};
    let (events, response_mode): (&[&str], _) = match name {
        "hello" => (&["hello"], Single),
        "list" => (&["sessions"], Single),
        "api_list" => (&["api_list"], Single),
        "api_describe" => (&["api_describe"], Single),
        "host_status" => (&["host_status"], Single),
        "restart_host" => (&["host_restarting"], Single),
        "session_exists" => (&["session_exists"], Single),
        "session_status" => (&["session_status"], Single),
        "spawn_status" => (&["spawn_status"], Single),
        "close_if_idle" => (&["closed", "close_blocked"], Single),
        "close_if_idle_batch" => (&["close_batch"], Single),
        "subscribe_sessions" => (
            &[
                "sessions",
                "session_added",
                "session_removed",
                "session_updated",
            ],
            Stream,
        ),
        "subscribe_events" => (
            &[
                "event_stream_started",
                "process_states",
                "event_stream_gap",
                "stream_event",
            ],
            Stream,
        ),
        "capture" => (&["captured"], Single),
        "attach" => (
            &[
                "attached",
                "snapshot",
                "binary_snapshot",
                "output",
                "closed",
            ],
            Stream,
        ),
        "detach" => (&["detached"], Single),
        "write" | "resize" => (&["done"], AckOptional),
        "spawn" => (&["spawned"], Single),
        "close" => (&["closed"], Single),
        "split_pane" => (&["pane_split"], Single),
        "layout" => (&["layout"], Single),
        "binary_snapshot" => (&["binary_snapshot"], Single),
        "set_access_scope"
        | "external_action"
        | "resize_pane"
        | "focus_pane"
        | "resize_window"
        | "report_program_status" => (&["done"], Single),
        _ => {
            return Err(format!(
                "the API catalog has no response mapping for {name}"
            ))
        }
    };
    Ok(ApiMethod {
        name: name.to_string(),
        description: request_schema["description"]
            .as_str()
            .unwrap_or(name)
            .to_string(),
        response_events: events
            .iter()
            .copied()
            .chain(["error"])
            .map(str::to_string)
            .collect(),
        response_mode,
    })
}

pub fn api_list() -> Result<ApiCatalog, String> {
    let requests = tagged_variants(&schema::<crate::daemon::Request>()?, "cmd")?;
    let methods = requests
        .iter()
        .map(|(name, request)| method(name, request))
        .collect::<Result<_, _>>()?;
    Ok(ApiCatalog {
        server: server(),
        methods,
    })
}

fn standalone_schema(document: &Value, variants: Vec<Value>) -> Value {
    json!({
        "$schema": document["$schema"],
        "oneOf": variants,
        "definitions": document["definitions"],
    })
}

pub fn api_describe(name: &str) -> Result<ApiMethodDescription, String> {
    let request_document = schema::<crate::daemon::Request>()?;
    let requests = tagged_variants(&request_document, "cmd")?;
    let request = requests
        .get(name)
        .ok_or_else(|| format!("unknown API method: {name}"))?;
    let method = method(name, request)?;
    let response_document = schema::<crate::daemon::Response>()?;
    let responses = tagged_variants(&response_document, "event")?;
    let result_variants = method
        .response_events
        .iter()
        .map(|event| {
            responses
                .get(event)
                .cloned()
                .ok_or_else(|| format!("the API response schema has no {event} event"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ApiMethodDescription {
        server: server(),
        method,
        request_schema: standalone_schema(&request_document, vec![request.clone()]),
        response_schema: standalone_schema(&response_document, result_variants),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_request_has_a_complete_response_schema() {
        let requests =
            tagged_variants(&schema::<crate::daemon::Request>().unwrap(), "cmd").unwrap();
        let catalog = api_list().unwrap();
        assert_eq!(catalog.methods.len(), requests.len());
        for method in catalog.methods {
            let description = api_describe(&method.name).unwrap();
            let request_variants = tagged_variants(&description.request_schema, "cmd").unwrap();
            assert_eq!(
                request_variants.keys().collect::<Vec<_>>(),
                vec![&method.name]
            );
            let responses = tagged_variants(&description.response_schema, "event").unwrap();
            assert_eq!(responses.len(), method.response_events.len());
            assert!(responses.contains_key("error"));
        }
    }

    #[test]
    fn describe_uses_the_protocol_fields_and_defaults() {
        let description = api_describe("attach").unwrap();
        let request = &description.request_schema["oneOf"][0];
        assert!(request["properties"]
            .get("expected_host_instance")
            .is_some());
        assert_eq!(request["properties"]["binary_snapshot"]["type"], "boolean");
        let parsed: crate::daemon::Request =
            serde_json::from_value(json!({"cmd":"attach", "id":1})).unwrap();
        assert!(matches!(
            parsed,
            crate::daemon::Request::Attach {
                binary_snapshot: false,
                ..
            }
        ));
        assert!(description.request_schema["definitions"]
            .get("AttachMode")
            .is_some());
        let required = request["required"].as_array().unwrap();
        assert!(required.contains(&json!("id")));
        assert!(!required.contains(&json!("mode")));
    }

    #[test]
    fn describe_rejects_unknown_methods() {
        assert_eq!(
            api_describe("unknown").unwrap_err(),
            "unknown API method: unknown"
        );
    }
}
