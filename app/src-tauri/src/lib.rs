mod cdp;

pub fn run() {
    tauri::Builder::default()
        .manage(cdp::init_state())
        .invoke_handler(tauri::generate_handler![
            cdp::commands::cdp_connect,
            cdp::commands::cdp_disconnect,
            cdp::commands::cdp_status,
            cdp::commands::cdp_refresh,
            cdp::commands::cdp_list_pages,
            cdp::commands::cdp_test_search,
        ])
        .run(tauri::generate_context!())
        .expect("error while running socai");
}
