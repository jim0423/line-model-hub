//! LINE Model Hub — provider-agnostic AI core
//!
//! Layering:
//!   - [`provider`] : Provider trait + concrete OpenAI / MiniMax / Anthropic / Ollama
//!   - [`sse`] : SSE wire-format parsing
//!   - [`conversation`] : agentic loop + send-guard
//!   - [`mcp`] : JSON-RPC client to line-desktop-mcp child
//!   - [`config`] : API key storage (keyring / encrypted file)

pub mod config;
pub mod conversation;
pub mod error;
pub mod mcp;
pub mod provider;
pub mod sse;

pub use error::{HubError, HubResult};