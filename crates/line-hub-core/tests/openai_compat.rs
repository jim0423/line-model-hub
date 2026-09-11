//! Wire-protocol round-trip tests for OpenAI-compat providers.

use line_hub_core::provider::openai_compat::{
    WireChatRequest, WireChoice, WireChunk, WireDelta, WireFunction, WireTool, WireToolCall,
    WireToolFn, WireUsage,
};
use line_hub_core::provider::{ChatRequest, Message, ToolDefinition};

#[test]
fn wire_chat_request_serializes_without_tools() {
    let req = WireChatRequest {
        model: "MiniMax-M3",
        messages: &vec![Message::User {
            content: "ping".into(),
        }],
        tools: vec![],
        tool_choice: None,
        temperature: 0.3,
        max_tokens: Some(2048),
        stream: true,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""model":"MiniMax-M3""#));
    assert!(json.contains(r#""stream":true"#));
    // `tools` is `skip_serializing_if=Vec::is_empty` so should not appear
    assert!(!json.contains("\"tools\""));
    // `tool_choice` is None → skipped
    assert!(!json.contains("tool_choice"));
}

#[test]
fn wire_chat_request_serializes_with_tool_choice_auto() {
    let req = WireChatRequest {
        model: "MiniMax-M3",
        messages: &vec![],
        tools: vec![WireTool {
            kind: "function",
            function: WireToolFn {
                name: "get_weather".into(),
                description: "Get weather".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"city": {"type": "string"}},
                    "required": ["city"]
                }),
            },
        }],
        tool_choice: Some("auto"),
        temperature: 0.3,
        max_tokens: None,
        stream: true,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""tools":[{"type":"function""#));
    assert!(json.contains(r#""tool_choice":"auto""#));
}

#[test]
fn wire_chunk_parses_minimax_thinking_response() {
    // Real chunk captured from the live API
    let raw = r#"{
        "choices":[{
            "delta":{
                "content":"<think>\nReasoning here.\n</think>\n\nanswer",
                "reasoning":"reasoning chunk",
                "tool_calls":null
            },
            "finish_reason":"stop",
            "index":0
        }],
        "usage":{"prompt_tokens":42,"completion_tokens":7}
    }"#;
    let chunk: WireChunk = serde_json::from_str(raw).unwrap();
    assert_eq!(chunk.choices.len(), 1);
    let choice: &WireChoice = &chunk.choices[0];
    assert_eq!(choice.delta.content.as_deref(), Some("<think>\nReasoning here.\n</think>\n\nanswer"));
    assert_eq!(choice.delta.reasoning.as_deref(), Some("reasoning chunk"));
    assert!(choice.delta.tool_calls.is_none());
}

#[test]
fn wire_chunk_parses_tool_call_index() {
    let raw = r#"{
        "choices":[{
            "delta":{
                "tool_calls":[{
                    "id":"call_abc",
                    "index":0,
                    "function":{"name":"get_weather","arguments":"{\"city\":\"Taipei\"}"}
                }]
            }
        }]
    }"#;
    let chunk: WireChunk = serde_json::from_str(raw).unwrap();
    let tc: &WireToolCall = &chunk.choices[0].delta.tool_calls.as_ref().unwrap()[0];
    assert_eq!(tc.id.as_deref(), Some("call_abc"));
    assert_eq!(tc.index, Some(0));
    let f: &WireFunction = tc.function.as_ref().unwrap();
    assert_eq!(f.name.as_deref(), Some("get_weather"));
    assert_eq!(f.arguments.as_deref(), Some(r#"{"city":"Taipei"}"#));
}

#[test]
fn usage_chunk_parses() {
    let raw = r#"{"usage":{"prompt_tokens":100,"completion_tokens":50}}"#;
    let chunk: WireChunk = serde_json::from_str(raw).unwrap();
    let u: &WireUsage = chunk.usage.as_ref().unwrap();
    assert_eq!(u.prompt_tokens, Some(100));
    assert_eq!(u.completion_tokens, Some(50));
}

#[test]
fn chat_request_accepts_all_send_types() {
    // Sanity: build a request with all send-style tools registered.
    let tools = vec![
        ToolDefinition {
            name: "send_message_auto".into(),
            description: "send".into(),
            parameters: serde_json::json!({"type":"object"}),
        },
        ToolDefinition {
            name: "get_line_chat_messages".into(),
            description: "read".into(),
            parameters: serde_json::json!({"type":"object"}),
        },
    ];
    let req = ChatRequest {
        model: "MiniMax-M3".into(),
        messages: vec![Message::User {
            content: "go".into(),
        }],
        tools,
        temperature: 0.2,
        max_tokens: Some(1500),
    };
    let wire = WireChatRequest {
        model: &req.model,
        messages: &req.messages,
        tools: req
            .tools
            .iter()
            .map(|t| WireTool {
                kind: "function",
                function: WireToolFn {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect(),
        tool_choice: Some("auto"),
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        stream: true,
    };
    let json = serde_json::to_string(&wire).unwrap();
    assert!(json.contains("send_message_auto"));
    assert!(json.contains("get_line_chat_messages"));
    assert!(json.contains(r#""tool_choice":"auto""#));
    assert!(json.contains(r#""temperature":0.2"#));
}