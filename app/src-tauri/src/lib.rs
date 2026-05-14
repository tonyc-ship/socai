mod commands;

use socai_browser::{BrowserEvent, Cdp};
use tauri::{Emitter, Manager};

pub fn run() {
    tauri::Builder::default()
        .manage(Cdp::new())
        .setup(|app| {
            let cdp = app.state::<Cdp>().inner().clone();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut rx = cdp.subscribe();
                while let Ok(event) = rx.recv().await {
                    match event {
                        BrowserEvent::StatusChanged(payload) => {
                            let _ = handle.emit("cdp:status_changed", payload);
                        }
                        BrowserEvent::TargetsChanged(pages) => {
                            let _ = handle.emit("cdp:targets_changed", pages);
                        }
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::cdp_connect,
            commands::cdp_disconnect,
            commands::cdp_status,
            commands::cdp_refresh,
            commands::cdp_list_pages,
            commands::cdp_test_search,
        ])
        .run(tauri::generate_context!())
        .expect("error while running socai");
}
