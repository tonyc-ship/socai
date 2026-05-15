pub mod connection;
pub mod endpoint;
pub mod lifecycle;
pub mod pages;
pub mod session;

pub use self::connection::{BrowserEvent, Cdp, CdpState, StatusPayload, TargetInfo};
pub use self::endpoint::{
    discover_existing_chrome_endpoint, open_remote_debugging_page, resolve_explicit_endpoint,
    wait_for_existing_chrome_endpoint, Endpoint,
};
pub use self::pages::PageSessionManager;
pub use self::session::PageSession;
