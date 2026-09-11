//! The wire shapes.
//!
//! One struct goes out, one comes in. The protocol multiplexes replies and
//! events onto the same stream and tells them apart by whether `id` is present,
//! so the incoming shape is deliberately permissive and sorted out after
//! decoding rather than by the deserializer.

use crate::error::ProtocolError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Identifies one attached target's session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A command on its way to the browser.
#[derive(Debug, Serialize)]
pub struct Command<'a> {
    pub id: u64,
    pub method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<&'a Value>,
    /// Present when the command is addressed to an attached session.
    ///
    /// A **top-level** field, which is the flattened protocol mode. The legacy
    /// alternative wrapped the whole message as a string inside a
    /// `Target.sendMessageToTarget` call. We always attach with `flatten: true`,
    /// so the legacy envelope never appears.
    #[serde(rename = "sessionId", skip_serializing_if = "Option::is_none")]
    pub session_id: Option<&'a SessionId>,
}

/// Anything arriving from the browser, before it is sorted into a reply or an
/// event.
#[derive(Debug, Deserialize)]
pub struct Incoming {
    /// Present on a reply, absent on an event. This is the only thing that
    /// distinguishes them.
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<WireError>,
    #[serde(rename = "sessionId", default)]
    pub session_id: Option<SessionId>,
}

/// The `error` object of a failed reply, as it appears on the wire.
#[derive(Debug, Clone, Deserialize)]
pub struct WireError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<String>,
}

impl From<WireError> for ProtocolError {
    fn from(wire: WireError) -> Self {
        ProtocolError {
            code: wire.code,
            message: wire.message,
            data: wire.data,
        }
    }
}

/// Something the browser reported without being asked.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// Fully qualified, such as `Page.loadEventFired`.
    pub method: String,
    pub params: Value,
    /// Set when the event belongs to an attached session rather than to the
    /// browser itself.
    pub session_id: Option<SessionId>,
}

impl Event {
    /// The domain half of the method name, such as `Page`.
    pub fn domain(&self) -> &str {
        self.method
            .split_once('.')
            .map_or(&*self.method, |(d, _)| d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_command_without_params_or_session_stays_minimal() {
        let command = Command {
            id: 1,
            method: "Page.enable",
            params: None,
            session_id: None,
        };
        let encoded = serde_json::to_string(&command).unwrap();
        assert_eq!(encoded, r#"{"id":1,"method":"Page.enable"}"#);
    }

    #[test]
    fn a_session_id_is_a_top_level_field_not_an_envelope() {
        let session = SessionId::new("ABC123");
        let params = json!({ "url": "https://example.com" });
        let command = Command {
            id: 7,
            method: "Page.navigate",
            params: Some(&params),
            session_id: Some(&session),
        };
        let encoded: Value =
            serde_json::from_str(&serde_json::to_string(&command).unwrap()).unwrap();
        assert_eq!(encoded["sessionId"], "ABC123");
        assert_eq!(encoded["method"], "Page.navigate");
        assert_eq!(encoded["params"]["url"], "https://example.com");
    }

    #[test]
    fn a_reply_is_told_from_an_event_by_the_id() {
        let reply: Incoming =
            serde_json::from_str(r#"{"id":1,"result":{"frameId":"F1"}}"#).unwrap();
        assert_eq!(reply.id, Some(1));
        assert!(reply.method.is_none());

        let event: Incoming =
            serde_json::from_str(r#"{"method":"Page.loadEventFired","params":{"timestamp":1.5}}"#)
                .unwrap();
        assert!(event.id.is_none());
        assert_eq!(event.method.as_deref(), Some("Page.loadEventFired"));
    }

    #[test]
    fn an_error_reply_decodes_into_a_protocol_error() {
        let reply: Incoming = serde_json::from_str(
            r#"{"id":2,"error":{"code":-32000,"message":"Cannot navigate","data":"bad url"}}"#,
        )
        .unwrap();
        let error: ProtocolError = reply.error.unwrap().into();
        assert_eq!(error.code, -32000);
        assert_eq!(error.message, "Cannot navigate");
        assert_eq!(error.data.as_deref(), Some("bad url"));
    }

    #[test]
    fn unknown_fields_do_not_break_decoding() {
        // The protocol gains fields between Chromium releases. Pinning a version
        // does not mean we should be brittle about it.
        let reply: Incoming =
            serde_json::from_str(r#"{"id":1,"result":{},"somethingNew":true}"#).unwrap();
        assert_eq!(reply.id, Some(1));
    }

    #[test]
    fn event_domain_is_the_half_before_the_dot() {
        let event = Event {
            method: "Network.requestWillBeSent".into(),
            params: json!({}),
            session_id: None,
        };
        assert_eq!(event.domain(), "Network");
    }
}
