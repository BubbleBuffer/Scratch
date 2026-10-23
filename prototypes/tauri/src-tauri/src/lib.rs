//! Rust side of the Tauri bake-off prototype: a thin layer of Tauri commands
//! over `vault_core` (through `backend::Backend`, which the headless test
//! bridge shares). Every vault call runs on Tauri's blocking thread pool
//! (`spawn_blocking`), never on the main (UI) thread.

pub mod backend;

use std::sync::Arc;

use backend::{Backend, OpenInfo, VaultStatus};
use tauri::{Manager, RunEvent, State, WindowEvent};
use vault_core::{BlockOp, Page, PageSummary, SearchHit};

type St<'a> = State<'a, Arc<Backend>>;

/// Runs `f` on the blocking pool and waits for it without holding a thread.
async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn vault_status(state: St<'_>) -> Result<VaultStatus, String> {
    Ok(state.status())
}

#[tauri::command]
async fn open_vault(state: St<'_>) -> Result<OpenInfo, String> {
    let b = Arc::clone(&state);
    blocking(move || b.open()).await
}

#[tauri::command]
async fn page_count(state: St<'_>) -> Result<u32, String> {
    let v = state.vault()?;
    blocking(move || v.page_count().map_err(String::from)).await
}

#[tauri::command]
async fn list_pages(state: St<'_>, offset: u32, limit: u32) -> Result<Vec<PageSummary>, String> {
    let v = state.vault()?;
    blocking(move || v.list_pages(None, offset, limit).map_err(String::from)).await
}

#[tauri::command]
async fn search(state: St<'_>, query: String, limit: u32) -> Result<Vec<SearchHit>, String> {
    let v = state.vault()?;
    blocking(move || v.search(query, limit).map_err(String::from)).await
}

#[tauri::command]
async fn create_page(state: St<'_>, title: String) -> Result<String, String> {
    let v = state.vault()?;
    blocking(move || v.create_page(None, title).map_err(String::from)).await
}

#[tauri::command]
async fn rename_page(state: St<'_>, id: String, title: String) -> Result<(), String> {
    let v = state.vault()?;
    blocking(move || v.rename_page(id, title).map_err(String::from)).await
}

#[tauri::command]
async fn load_page(state: St<'_>, id: String) -> Result<Page, String> {
    let v = state.vault()?;
    blocking(move || v.load_page(id).map_err(String::from)).await
}

#[tauri::command]
async fn apply(state: St<'_>, id: String, ops: Vec<BlockOp>) -> Result<(), String> {
    let v = state.vault()?;
    blocking(move || v.apply(id, ops).map_err(String::from)).await
}

#[tauri::command]
async fn flush(state: St<'_>) -> Result<(), String> {
    let Ok(v) = state.vault() else { return Ok(()) };
    blocking(move || v.flush().map_err(String::from)).await
}

/// One measurement line from the page (see `Backend::log`).
#[tauri::command]
fn bakeoff_log(state: St<'_>, line: String) {
    state.log(&line);
}

/// Flushes the vault's background writer on a worker thread.
fn flush_in_background(b: &Backend) {
    if let Ok(v) = b.vault() {
        std::thread::spawn(move || {
            let _ = v.flush();
        });
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let backend = Arc::new(Backend::new(dir));
            backend.log("app_setup");
            app.manage(backend);
            // Match the window background to the system theme so a dark
            // system sees no white flash before the page paints.
            //
            // The window starts hidden (tauri.conf.json) and is shown here, so
            // NOTES_TAURI_HIDDEN=1 can run the full app with no window on
            // screen (the headless start check in the bake-off).
            #[cfg(desktop)]
            if let Some(w) = app.get_webview_window("main") {
                if let Ok(tauri::Theme::Dark) = w.theme() {
                    let _ = w.set_background_color(Some(tauri::window::Color(25, 25, 25, 255)));
                }
                if std::env::var_os("NOTES_TAURI_HIDDEN").is_none() {
                    let _ = w.show();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Losing focus is the desktop's "pause": flush pending writes.
            if let WindowEvent::Focused(false) = event {
                flush_in_background(&window.state::<Arc<Backend>>());
            }
        })
        .invoke_handler(tauri::generate_handler![
            vault_status,
            open_vault,
            page_count,
            list_pages,
            search,
            create_page,
            rename_page,
            load_page,
            apply,
            flush,
            bakeoff_log
        ])
        .build(tauri::generate_context!())
        .expect("error while building the Tauri app");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            if let Ok(v) = handle.state::<Arc<Backend>>().vault() {
                let _ = v.flush();
            }
        }
    });
}
