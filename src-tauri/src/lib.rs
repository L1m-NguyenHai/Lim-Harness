mod catalog;
mod cookbook;
mod memory;
mod memory_runtime;
mod runtime;
mod storage;
mod vault;
mod vault_setup;
use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let directory = app.path().app_data_dir()?;
            // Debug smoke tests use a separate database without changing the user's app data.
            #[cfg(debug_assertions)]
            let directory = match std::env::var_os("LIM_HARNESS_TEST_DATA") {
                Some(path) => {
                    let path = std::path::PathBuf::from(path);
                    if !path.is_absolute() {
                        return Err(std::io::Error::other(
                            "LIM_HARNESS_TEST_DATA must be absolute",
                        )
                        .into());
                    }
                    path
                }
                None => directory,
            };
            std::fs::create_dir_all(&directory)?;
            let store = storage::Store::open(&directory.join("harness.sqlite3"))
                .map_err(std::io::Error::other)?;
            let runtime = runtime::Runtime::new(store).map_err(std::io::Error::other)?;
            let runtime = Arc::new(runtime);
            memory_runtime::start_worker(Arc::clone(&runtime));
            app.manage(runtime);
            app.manage(Arc::new(cookbook::Cookbook::new()));
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
            vault_setup::initialize_memory_vault,
            vault_setup::open_memory_obsidian,
            runtime::search_memory,
            memory_runtime::get_memory_snapshot,
            memory_runtime::configure_memory,
            memory_runtime::create_memory,
            memory_runtime::update_memory,
            memory_runtime::read_memory,
            memory_runtime::retry_memory_job,
            runtime::list_models,
            runtime::start_turn,
            runtime::continue_turn,
            runtime::cancel_turn,
            cookbook::cookbook_snapshot,
            cookbook::start_model_setup,
            cookbook::cancel_model_setup,
            cookbook::unload_model,
            cookbook::delete_model,
            catalog::get_model_catalog,
            catalog::search_model_catalog
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Lim Harness");
}
