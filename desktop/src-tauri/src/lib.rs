mod ffmpeg;
mod ltc;
mod render;

use render::RenderOptions;

#[tauri::command]
fn check_ffmpeg() -> Result<String, String> {
    ffmpeg::check_available()
}

#[tauri::command]
fn probe_video(path: String) -> Result<String, String> {
    ffmpeg::probe(&path)
}

/// 書き出しは数十秒かかることがあるため、UIを止めないよう別スレッドで実行する
#[tauri::command]
async fn render_video(options: RenderOptions) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || render::render(options))
        .await
        .map_err(|e| format!("処理を実行できませんでした: {e}"))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            check_ffmpeg,
            probe_video,
            render_video
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
