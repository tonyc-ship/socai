pub mod endpoint;
pub mod page;
pub mod state;
pub mod supervisor;
pub mod task;

pub use endpoint::{
    discover_existing_chrome_endpoint, open_remote_debugging_page, resolve_explicit_endpoint,
    wait_for_existing_chrome_endpoint, Endpoint,
};
pub use page::PageSession;
pub use state::{BrowserEvent, Cdp, CdpState, StatusPayload, TargetInfo};
pub use task::PageSessionManager;
