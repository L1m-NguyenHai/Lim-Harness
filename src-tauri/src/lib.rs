mod agent;
mod catalog;
mod cookbook;
mod runtime;
mod searxng;
mod storage;
mod vault;
use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let directory = app.path().app_data_dir()?;
            std::fs::create_dir_all(&directory)?;
            let store = storage::Store::open(&directory.join("harness.sqlite3"))
                .map_err(std::io::Error::other)?;
            let runtime = runtime::Runtime::new(store).map_err(std::io::Error::other)?;
            app.manage(Arc::new(runtime));
            app.manage(Arc::new(cookbook::Cookbook::new()));
            // Local SearXNG for agent web search lives as long as the app (see searxng.rs).
            std::thread::spawn(searxng::start);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            runtime::get_snapshot,
            runtime::create_session,
            runtime::delete_session,
            runtime::set_session_config,
            runtime::import_sessions,
            runtime::set_model,
            runtime::set_inference_settings,
            runtime::set_context_size,
            runtime::set_vault,
            runtime::search_memory,
            runtime::list_models,
            runtime::start_turn,
            runtime::continue_turn,
            runtime::resolve_approval,
            runtime::compact_session,
            runtime::model_context,
            runtime::cancel_turn,
            cookbook::cookbook_snapshot,
            cookbook::start_model_setup,
            cookbook::cancel_model_setup,
            cookbook::unload_model,
            catalog::get_model_catalog,
            catalog::search_model_catalog
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Lim Harness")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                searxng::stop();
            }
        });
}
