pub mod entities;
pub mod page;
pub mod tools;

pub use self::entities::{parse_count_text, XhsAuthorProfile, XhsNote, XhsNoteCard};
pub use self::page::{ReadNoteOptions, XhsPageRuntime, XHS_HOME_URL};
pub use self::tools::{
    close_open_note, ensure_search_ready, extract_note_command, search_notes_command,
    topic_scan_command, xhs_agent_instructions, xhs_agent_tools, xhs_tools,
    xhs_tools_with_llm_provider, XHS_AGENT_HINT, XHS_BROWSER_LOCK_PROMPT,
};
