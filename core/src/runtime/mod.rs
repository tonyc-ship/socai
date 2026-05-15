mod engine;

pub use self::engine::{
    create_llm_provider, ensure_llm_provider_configured, resolve_llm_model, run_agent_task,
    wait_browser_connected, AgentRunConfig, SocaiRuntime,
};
pub use crate::cdp::{
    BrowserEvent as RuntimeBrowserEvent, PageSession as RuntimePageSession,
    StatusPayload as BrowserStatus, TargetInfo as BrowserTargetInfo,
};
