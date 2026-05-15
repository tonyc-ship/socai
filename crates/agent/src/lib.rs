//! Agent runtime: LLM clients, the Tool trait, the agent loop, and
//! run-state persistence.
//!
//! Browser/CDP is intentionally out of scope — site tools live in
//! `socai-sites` and call into `socai-browser` themselves.

pub mod api_errors;
pub mod compaction;
pub mod llm;
pub mod r#loop;
pub mod memory;
pub mod provider;
pub mod report;
pub mod run_logging;
pub mod run_state;
pub mod signature;
pub mod system_prompt;
pub mod tool;

pub use llm::{
    AnthropicBackend, Backend, Block, LLMResponse, Message, MessageContent, MessageRole,
    OpenAICompatBackend, StopReason, ToolCall, ToolResultContent, ToolSchema,
};
pub use r#loop::{run_agent, run_agent_with_events, AgentEvent, AgentOptions, AgentOutcome};
pub use provider::{
    default_model_for, list_available_providers, load_api_key, resolve_provider, save_api_key,
    Provider, ProviderConfig, PROVIDERS,
};
pub use run_logging::{make_run_dir, RunDebugLogger};
pub use run_state::{ArtifactRecord, RunState};
pub use tool::{EchoTool, SharedTool, Tool, ToolContext, ToolResult, ToolResultBlock};
