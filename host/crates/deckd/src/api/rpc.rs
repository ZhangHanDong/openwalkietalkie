//! The JSON-RPC 2.0 envelope (design §4.4, contract §3.3): requests,
//! batches, notifications, responses, the parse and invalid-request
//! errors, and the `event`/`resync` notifications the doors write.
use deck_model::{ApiError, Event};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const VERSION: &str = "2.0";

/// One request. `id` absent → a notification (executed, never answered);
/// `id` present (a number, a string, or `null`) → answered with that id.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct RpcRequest {
    #[serde(default)]
    pub jsonrpc: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// `null` stays `Some(Null)`: a present id, unlike an absent one.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

impl RpcRequest {
    pub fn is_notification(&self) -> bool {
        self.id.is_none()
    }

    /// The `client.name` of the params, when the call names one (§4.2).
    pub fn client_name(&self) -> Option<&str> {
        self.params.get("client")?.get("name")?.as_str()
    }
}

/// A line/frame/body: one request or a batch. Each batch member carries
/// its own parse verdict: JSON-RPC 2.0 answers a malformed member in
/// place (`-32600`, `id:null`) and runs the ones beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    Single(RpcRequest),
    Batch(Vec<Result<RpcRequest, RpcError>>),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
}

impl From<ApiError> for RpcError {
    fn from(e: ApiError) -> Self {
        RpcError {
            code: e.code,
            message: e.message,
            data: e.data,
        }
    }
}

impl From<RpcError> for ApiError {
    fn from(e: RpcError) -> Self {
        ApiError {
            code: e.code,
            message: e.message,
            data: e.data,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    pub fn ok(id: Value, result: Value) -> Self {
        RpcResponse {
            jsonrpc: VERSION.into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, error: impl Into<RpcError>) -> Self {
        RpcResponse {
            jsonrpc: VERSION.into(),
            id,
            result: None,
            error: Some(error.into()),
        }
    }

    pub fn from_result(id: Value, result: Result<Value, ApiError>) -> Self {
        match result {
            Ok(v) => RpcResponse::ok(id, v),
            Err(e) => RpcResponse::err(id, e),
        }
    }

    /// One line, no trailing newline.
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32603,\"message\":\"internal\"}}"
                .into()
        })
    }
}

/// Validate one parsed value as a request.
fn request_of(value: Value) -> Result<RpcRequest, RpcError> {
    if !value.is_object() {
        return Err(ApiError::invalid_request("a request is an object").into());
    }
    if let Some(id) = value.get("id") {
        if !(id.is_number() || id.is_string() || id.is_null()) {
            return Err(ApiError::invalid_request("id must be a number, a string or null").into());
        }
    }
    if !value.get("method").is_some_and(Value::is_string) {
        return Err(ApiError::invalid_request("method must be a string").into());
    }
    if let Some(v) = value.get("jsonrpc") {
        if v.as_str() != Some(VERSION) {
            return Err(ApiError::invalid_request("jsonrpc must be \"2.0\"").into());
        }
    }
    if value
        .get("params")
        .is_some_and(|p| !(p.is_object() || p.is_array() || p.is_null()))
    {
        return Err(ApiError::invalid_request("params must be an object or an array").into());
    }
    serde_json::from_value(value).map_err(|e| ApiError::invalid_request(&e.to_string()).into())
}

/// Parse a line, frame or body: `-32700` when it is not JSON, `-32600`
/// when it is not a request (an empty batch included). A batch itself is
/// never refused as a whole for one bad member: every member is returned
/// with its own verdict — `Err(-32600 {why, index})` for a malformed one,
/// which the door answers in place with `id:null` (JSON-RPC 2.0) while
/// the members beside it run.
pub fn parse(text: &str) -> Result<Incoming, RpcError> {
    let value: Value = serde_json::from_str(text).map_err(|e| ApiError::parse(&e.to_string()))?;
    match value {
        Value::Array(items) => {
            if items.is_empty() {
                return Err(ApiError::invalid_request("empty batch").into());
            }
            Ok(Incoming::Batch(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(index, item)| {
                        request_of(item).map_err(|mut e| {
                            e.data = serde_json::json!({"why": e.data["why"], "index": index});
                            e
                        })
                    })
                    .collect(),
            ))
        }
        other => request_of(other).map(Incoming::Single),
    }
}

/// `{"jsonrpc":"2.0","method":..,"params":..}` as one line.
pub fn notification(method: &str, params: Value) -> String {
    serde_json::json!({"jsonrpc": VERSION, "method": method, "params": params}).to_string()
}

/// An `event` notification.
pub fn event_line(event: &Event) -> String {
    notification("event", serde_json::to_value(event).unwrap_or(Value::Null))
}

/// A `resync` notification (the subscriber lagged: `state.get` again).
pub fn resync_line(rev: u64) -> String {
    notification("resync", serde_json::json!({"rev": rev}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_notifications_and_batches_parse_and_bad_input_is_the_standard_error() {
        let one = parse(r#"{"jsonrpc":"2.0","id":7,"method":"session.ping","params":{}}"#).unwrap();
        let Incoming::Single(req) = one else {
            panic!("{one:?}");
        };
        assert_eq!(req.id, Some(serde_json::json!(7)));
        assert!(!req.is_notification());
        assert_eq!(req.method, "session.ping");
        let note = parse(r#"{"method":"session.ping"}"#).unwrap();
        let Incoming::Single(note) = note else {
            panic!("{note:?}");
        };
        assert!(note.is_notification());
        assert_eq!(note.params, Value::Null);
        let null_id = parse(r#"{"id":null,"method":"x"}"#).unwrap();
        let Incoming::Single(null_id) = null_id else {
            panic!();
        };
        assert_eq!(
            null_id.id,
            Some(Value::Null),
            "a present null id is answered"
        );
        let string_id = parse(r#"{"id":"a","method":"x","params":[1]}"#).unwrap();
        assert!(matches!(string_id, Incoming::Single(r) if r.id == Some("a".into())));
        let batch =
            parse(r#"[{"id":1,"method":"a"},{"method":"b","params":{"client":{"name":"bot"}}}]"#)
                .unwrap();
        let Incoming::Batch(items) = batch else {
            panic!("{batch:?}");
        };
        let items: Vec<RpcRequest> = items.into_iter().map(Result::unwrap).collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].client_name(), Some("bot"));
        assert_eq!(items[0].client_name(), None);
        // Errors.
        assert_eq!(parse("not json").unwrap_err().code, ApiError::PARSE);
        assert_eq!(parse("[]").unwrap_err().code, ApiError::INVALID_REQUEST);
        assert_eq!(parse("42").unwrap_err().code, ApiError::INVALID_REQUEST);
        assert_eq!(
            parse(r#"{"id":1}"#).unwrap_err().code,
            ApiError::INVALID_REQUEST
        );
        assert_eq!(
            parse(r#"{"id":{"a":1},"method":"x"}"#).unwrap_err().code,
            ApiError::INVALID_REQUEST
        );
        assert_eq!(
            parse(r#"{"jsonrpc":"1.0","id":1,"method":"x"}"#)
                .unwrap_err()
                .code,
            ApiError::INVALID_REQUEST
        );
        assert_eq!(
            parse(r#"{"id":1,"method":"x","params":"s"}"#)
                .unwrap_err()
                .code,
            ApiError::INVALID_REQUEST
        );
        // One malformed member never refuses the batch (JSON-RPC 2.0):
        // it comes back as that member's own error, `index` and all.
        let bad_batch = parse(r#"[{"id":1,"method":"a"},{"id":2}]"#).unwrap();
        let Incoming::Batch(members) = bad_batch else {
            panic!("{bad_batch:?}");
        };
        assert_eq!(members[0].as_ref().unwrap().method, "a");
        let bad = members[1].as_ref().unwrap_err();
        assert_eq!(bad.code, ApiError::INVALID_REQUEST);
        assert_eq!(bad.data["index"], 1);
        // Every member may be malformed; each keeps its own index.
        let all_bad = parse(r#"[{"id":2},"nope"]"#).unwrap();
        let Incoming::Batch(members) = all_bad else {
            panic!("{all_bad:?}");
        };
        let indexes: Vec<Value> = members
            .iter()
            .map(|m| m.as_ref().unwrap_err().data["index"].clone())
            .collect();
        assert_eq!(indexes, vec![serde_json::json!(0), serde_json::json!(1)]);
        assert_eq!(
            members[1].as_ref().unwrap_err().data["why"],
            "a request is an object"
        );
    }

    #[test]
    fn responses_and_notifications_serialize_to_the_envelope() {
        let ok = RpcResponse::ok(serde_json::json!(7), serde_json::json!({"rev": 3}));
        assert_eq!(
            ok.to_line(),
            r#"{"jsonrpc":"2.0","id":7,"result":{"rev":3}}"#
        );
        let err = RpcResponse::err(Value::Null, ApiError::parse("bad"));
        assert_eq!(
            err.to_line(),
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse_error","data":{"why":"bad"}}}"#
        );
        let err = RpcResponse::from_result(serde_json::json!("a"), Err(ApiError::unauthorized()));
        let back: RpcResponse = serde_json::from_str(&err.to_line()).unwrap();
        assert_eq!(back.error.as_ref().unwrap().code, ApiError::UNAUTHORIZED);
        assert_eq!(back.id, serde_json::json!("a"));
        let api: ApiError = back.error.unwrap().into();
        assert_eq!(api.code, ApiError::UNAUTHORIZED);
        let line = notification("event", serde_json::json!({"type": "card.set"}));
        assert_eq!(
            line,
            r#"{"jsonrpc":"2.0","method":"event","params":{"type":"card.set"}}"#
        );
        assert_eq!(
            resync_line(9),
            r#"{"jsonrpc":"2.0","method":"resync","params":{"rev":9}}"#
        );
        let event = Event {
            rev: 1,
            ts: deck_model::Ts(0),
            kind: deck_model::EventKind::Error,
            principal: deck_model::Principal::Device,
            data: serde_json::json!({}),
        };
        let line: Value = serde_json::from_str(&event_line(&event)).unwrap();
        assert_eq!(line["method"], "event");
        assert_eq!(line["params"]["type"], "error");
        assert_eq!(line["params"]["rev"], 1);
    }
}
