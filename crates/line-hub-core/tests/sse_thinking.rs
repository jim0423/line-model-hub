//! Unit tests for the OpenAI-compat SSE parser and thinking-tag stripper.
//!
//! Run with `cargo test -p line-hub-core`.

use line_hub_core::provider::openai_compat::CompatStreamState;

#[test]
fn thinking_tag_is_stripped_from_delta() {
    let mut s = CompatStreamState::new(true, true);
    // First chunk: opens the tag and carries partial body
    let v1 = s.process_content_delta("<think>abc");
    assert_eq!(v1.as_deref(), None, "all content inside thinking");
    // Second chunk: closes the tag, leaves visible content
    let v2 = s.process_content_delta("</think>hi ");
    assert_eq!(v2.as_deref(), Some("hi "));
    // Third chunk
    let v3 = s.process_content_delta("there");
    assert_eq!(v3.as_deref(), Some("there"));
}

#[test]
fn no_thinking_strip_when_disabled() {
    let mut s = CompatStreamState::new(false, false);
    let v = s.process_content_delta("...abc</think>hi");
    assert_eq!(v.as_deref(), Some("...abc</think>hi"));
}

#[test]
fn mixed_tags_are_handled() {
    // MiniMax emits both <thinking> and <think> in the wild.
    let mut s = CompatStreamState::new(true, true);
    let v1 = s.process_content_delta("hello<think>internal</think> world");
    assert_eq!(v1.as_deref(), Some("hello world"));
}

#[test]
fn openai_style_works_when_disabled() {
    let mut s = CompatStreamState::new(false, false);
    assert_eq!(
        s.process_content_delta("Hello ").as_deref(),
        Some("Hello ")
    );
    assert_eq!(
        s.process_content_delta("world").as_deref(),
        Some("world")
    );
}

#[test]
fn unclosed_thinking_tag_is_carried_over() {
    let mut s = CompatStreamState::new(true, true);
    // First chunk has the opening but no closing
    let v1 = s.process_content_delta("<think>halfway");
    assert_eq!(v1.as_deref(), None);
    // Second chunk closes it and continues with visible text
    let v2 = s.process_content_delta("done</think>visible");
    assert_eq!(v2.as_deref(), Some("visible"));
}