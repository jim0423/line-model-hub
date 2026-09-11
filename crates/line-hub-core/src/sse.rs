//! SSE wire-format parsing helpers.
//!
//! We mostly delegate to `reqwest-eventsource` inside each provider, but this
//! module exposes utility types used by tests and the conversation loop.

use crate::HubError;
use bytes::Bytes;
use futures::stream::Stream;

/// Marker newtype around `EventSource` for clarity in trait signatures.
pub struct SseStream;

/// Parse a single SSE `data:` chunk payload as JSON.
pub fn parse_sse_data<T: serde::de::DeserializeOwned>(data: &str) -> Result<T, HubError> {
    let trimmed = data.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return Err(HubError::Config("empty SSE payload".into()));
    }
    serde_json::from_str(trimmed).map_err(HubError::Json)
}

/// Split an SSE byte stream into `data:` payloads (without the prefix).
pub fn data_payloads(buf: &mut Bytes, drain_to: usize) -> Vec<String> {
    let mut out = Vec::new();
    if buf.is_empty() {
        return out;
    }
    let s = std::str::from_utf8(buf).unwrap_or("");
    for line in s.split('\n').take(drain_to) {
        if let Some(rest) = line.strip_prefix("data: ") {
            out.push(rest.to_string());
        }
    }
    out
}

// Provide a no-op Stream impl so `use SseStream` is not flagged dead.
impl Stream for SseStream {
    type Item = Bytes;
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Ready(None)
    }
}