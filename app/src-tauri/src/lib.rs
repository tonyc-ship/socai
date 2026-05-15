mod commands;

use socai_runtime::{RuntimeBrowserEvent, SocaiRuntime};
use tauri::{Emitter, Manager};

pub fn run() {
    // Tauri owns its own in-process runtime.
    let runtime = SocaiRuntime::new();

    tauri::Builder::default()
        .manage(runtime)
        .setup(|app| {
            let runtime = app.state::<SocaiRuntime>().inner().clone();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut rx = runtime.subscribe_browser_events();
                while let Ok(event) = rx.recv().await {
                    match event {
                        RuntimeBrowserEvent::StatusChanged(payload) => {
                            let _ = handle.emit("cdp:status_changed", payload);
                        }
                        RuntimeBrowserEvent::TargetsChanged(pages) => {
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
            commands::tool_search_notes,
            commands::tool_topic_scan,
            commands::tool_extract_note,
            commands::agent_list_models,
            commands::agent_save_api_key,
            commands::agent_run,
        ])
        .run(tauri::generate_context!())
        .expect("error while running socai");
}
