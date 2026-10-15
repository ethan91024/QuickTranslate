mod config;
mod ocr;
mod screen;
mod translation;

use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use serde_json::json;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{
    LogicalSize, Manager, PhysicalPosition, Position, Size, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};
use tauri::webview::Color;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

const APP_VERSION: &str = "0.1.4";
const HOTKEY_COOLDOWN_MS: u64 = 1500;

static HOTKEY_DEBOUNCE: AtomicBool = AtomicBool::new(false);
static LAST_HOTKEY_TS: AtomicU64 = AtomicU64::new(0);
static CAPTURE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static SCREEN_CAPTURE_LOCK: Mutex<()> = Mutex::new(());
static HOTKEY_SERIALIZER: Mutex<()> = Mutex::new(());
static CURSOR_FOLLOWER_RUNNING: AtomicBool = AtomicBool::new(false);
static OVERLAY_DISMISSED: AtomicBool = AtomicBool::new(false);
static MOUSE_SIDE_TRIGGER: AtomicU8 = AtomicU8::new(0);

struct SingleInstanceGuard {
    path: std::path::PathBuf,
    _file: std::fs::File,
}

impl SingleInstanceGuard {
    fn acquire() -> Result<Self, String> {
        let path = std::env::temp_dir().join("quicktranslate.lock");

        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }

        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|err| format!("Failed to create single-instance lock: {err}"))?;

        Ok(Self {
            path,
            _file: file,
        })
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[tauri::command]
fn load_settings() -> Result<serde_json::Value, String> {
    let settings = config::Settings::load()?;
    Ok(serde_json::json!(settings))
}

#[tauri::command]
fn save_settings(
    app: tauri::AppHandle,
    api_key: Option<String>,
    base_url: String,
    model: String,
    shortcut: String,
    target_language: String,
    source_language: String,
    interface_language: String,
) -> Result<serde_json::Value, String> {
    let previous_settings = config::Settings::load().ok();
    let settings = config::Settings {
        api_key: api_key.unwrap_or_default(),
        base_url,
        model,
        shortcut,
        target_language,
        source_language,
        interface_language,
    };

    let shortcut_changed = previous_settings
        .as_ref()
        .is_some_and(|previous| previous.shortcut != settings.shortcut);
    if shortcut_changed {
        let previous_shortcut = previous_settings.as_ref().map(|previous| previous.shortcut.as_str());
        register_translation_shortcut(&app, &settings.shortcut, previous_shortcut)?;
    }

    if let Err(err) = settings.save() {
        if shortcut_changed {
            if let Some(previous) = &previous_settings {
                let _ = register_translation_shortcut(&app, &previous.shortcut, None);
            }
        }
        return Err(err);
    }

    update_native_language(&app, &settings.interface_language);

    Ok(serde_json::json!(settings))
}

#[derive(Clone)]
struct TrayMenuItems {
    settings: MenuItem<tauri::Wry>,
    toggle: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

#[derive(Clone, Copy)]
enum OverlayStatus {
    Translating,
    CaptureFailed,
    NoText,
    TranslationFailed,
}

fn overlay_status_text(language: &str, status: OverlayStatus) -> &'static str {
    match (language, status) {
        ("zh-Hans", OverlayStatus::CaptureFailed) => "屏幕识别失败",
        ("zh-Hans", OverlayStatus::NoText) => "未识别到文字",
        ("zh-Hans", OverlayStatus::TranslationFailed) => "翻译失败",
        ("en", OverlayStatus::CaptureFailed) => "Screen recognition failed",
        ("en", OverlayStatus::NoText) => "No text detected",
        ("en", OverlayStatus::TranslationFailed) => "Translation failed",
        ("ja", OverlayStatus::CaptureFailed) => "画面を認識できませんでした",
        ("ja", OverlayStatus::NoText) => "テキストが見つかりません",
        ("ja", OverlayStatus::TranslationFailed) => "翻訳に失敗しました",
        ("ko", OverlayStatus::CaptureFailed) => "화면 인식에 실패했습니다",
        ("ko", OverlayStatus::NoText) => "텍스트를 찾을 수 없습니다",
        ("ko", OverlayStatus::TranslationFailed) => "번역에 실패했습니다",
        (_, OverlayStatus::Translating) => "",
        (_, OverlayStatus::CaptureFailed) => "畫面辨識失敗",
        (_, OverlayStatus::NoText) => "沒有辨識到文字",
        (_, OverlayStatus::TranslationFailed) => "翻譯失敗",
    }
}

fn settings_window_title(language: &str) -> &'static str {
    match language {
        "zh-Hans" => "QuickTranslate 设置",
        "en" => "QuickTranslate Settings",
        "ja" => "QuickTranslate 設定",
        "ko" => "QuickTranslate 설정",
        _ => "QuickTranslate 設定",
    }
}

fn tray_labels(language: &str) -> (&'static str, &'static str, &'static str) {
    match language {
        "zh-Hans" => ("设置", "切换翻译", "退出 QuickTranslate"),
        "en" => ("Settings", "Toggle translation", "Quit QuickTranslate"),
        "ja" => ("設定", "翻訳の切り替え", "QuickTranslate を終了"),
        "ko" => ("설정", "번역 전환", "QuickTranslate 종료"),
        _ => ("設定", "切換翻譯", "結束 QuickTranslate"),
    }
}

fn update_native_language(app: &tauri::AppHandle, language: &str) {
    if let Some(items) = app.try_state::<TrayMenuItems>() {
        let (settings, toggle, quit) = tray_labels(language);
        let _ = items.settings.set_text(settings);
        let _ = items.toggle.set_text(toggle);
        let _ = items.quit.set_text(quit);
    }

    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.set_title(settings_window_title(language));
    }
}

fn show_overlay_status(app: &tauri::AppHandle, language: &str, status: OverlayStatus) {
    let app = app.clone();
    let app_for_overlay = app.clone();
    let text = overlay_status_text(language, status).to_string();
    let is_loading = matches!(status, OverlayStatus::Translating);
    let _ = app.run_on_main_thread(move || {
        let _ = show_overlay_content(app_for_overlay, text, is_loading);
    });
}

fn register_translation_shortcut(
    app: &tauri::AppHandle,
    shortcut: &str,
    fallback: Option<&str>,
) -> Result<(), String> {
    app.global_shortcut()
        .unregister_all()
        .map_err(|err| format!("Failed to unregister previous shortcut: {err}"))?;

    let mouse_button = shortcut
        .strip_prefix("Mouse")
        .and_then(|button| button.parse::<u8>().ok())
        .filter(|button| (1..=5).contains(button));
    if let Some(button) = mouse_button {
        MOUSE_SIDE_TRIGGER.store(button, Ordering::SeqCst);
        return Ok(());
    }

    MOUSE_SIDE_TRIGGER.store(0, Ordering::SeqCst);

    let registration = app.global_shortcut().on_shortcut(shortcut, |_app, _shortcut, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            handle_translation_shortcut(_app.clone());
        }
    });

    if let Err(err) = registration {
        if let Some(fallback) = fallback.filter(|fallback| *fallback != shortcut) {
            let _ = register_translation_shortcut(app, fallback, None);
        }
        return Err(format!("Failed to register shortcut '{shortcut}': {err}"));
    }

    Ok(())
}

#[tauri::command]
fn ping() -> String {
    "Rust is working!".to_string()
}

#[tauri::command]
fn quick_capture() -> Result<serde_json::Value, String> {
    let (x, y) = screen::get_cursor_position()?;
    let image = screen::capture_primary_screen()?;
    let region_width = image.width().min(1200);
    let region_height = image.height().min(360);
    let region_left = (x - region_width as i32 / 2).clamp(0, image.width() as i32 - region_width as i32);
    let region_top = (y - region_height as i32 / 2).clamp(0, image.height() as i32 - region_height as i32);
    let region = image::imageops::crop_imm(
        &image,
        region_left as u32,
        region_top as u32,
        region_width,
        region_height,
    )
    .to_image();
    let ocr_result = ocr::recognize(&region)?;
    let selected_text = ocr::find_word_under_cursor(
        &ocr_result.words,
        x - region_left,
        y - region_top,
    )
    .unwrap_or_default();

    Ok(json!({
        "x": x,
        "y": y,
        "ocrText": ocr_result.raw_text,
        "selectedText": selected_text,
        "words": ocr_result.words,
    }))
}

#[tauri::command]
fn ensure_overlay_window(app: &tauri::AppHandle) -> Result<(), String> {
    if app.get_webview_window("translation-overlay").is_some() {
        return Ok(());
    }

    let overlay = WebviewWindowBuilder::new(
        app,
        "translation-overlay",
        WebviewUrl::App("overlay.html".into()),
    )
    .title("QuickTranslate Overlay")
    .transparent(true)
    .no_redirection_bitmap(true)
    .background_color(Color(0, 0, 0, 0))
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(false)
    .focusable(false)
    .inner_size(420.0, 180.0)
    .visible(false)
    .build()
    .map_err(|err| format!("Failed to create overlay window: {err}"))?;

    overlay.set_ignore_cursor_events(false).map_err(|err| format!("Failed to allow overlay interactions: {err}"))?;
    Ok(())
}

fn show_translation_overlay(app: tauri::AppHandle, text: String) -> Result<(), String> {
    show_overlay_content(app, text, false)
}

fn overlay_window_size(text: &str, is_loading: bool) -> (f64, f64) {
    if is_loading {
        return (44.0, 32.0);
    }

    let line_widths: Vec<f64> = text
        .lines()
        .map(|line| {
            line.chars()
                .map(|character| if character.is_ascii() { 7.0 } else { 12.0 })
                .sum::<f64>()
        })
        .collect();
    let line_count: usize = line_widths
        .iter()
        .map(|width| (width / 190.0).ceil().max(1.0) as usize)
        .sum::<usize>()
        .max(1);
    let width = (line_widths.into_iter().fold(0.0_f64, f64::max).min(190.0) + 28.0)
        .clamp(120.0, 220.0);
    let height = (line_count as f64 * 15.0 + 10.0).clamp(32.0, 240.0);

    (width, height)
}

fn show_overlay_content(
    app: tauri::AppHandle,
    text: String,
    is_loading: bool,
) -> Result<(), String> {
    let text = text.trim_start().to_string();

    ensure_overlay_window(&app)?;

    let overlay = app.get_webview_window("translation-overlay").ok_or_else(|| "Overlay window not found after creation".to_string())?;
    let _ = overlay.hide();
    overlay.set_ignore_cursor_events(false).map_err(|err| format!("Failed to allow overlay interactions: {err}"))?;

    let (width, height) = overlay_window_size(&text, is_loading);
    overlay
        .set_size(Size::Logical(LogicalSize::new(width, height)))
        .map_err(|err| format!("Failed to size overlay: {err}"))?;

    let payload = json!({ "text": text, "loading": is_loading });
    let payload_json = serde_json::to_string(&payload)
        .map_err(|err| format!("Failed to serialize overlay payload: {err}"))?;

    let script = format!(
        r#"(function() {{
            const payload = {payload_json};
            const overlay = document.getElementById('overlay');
            const textEl = document.getElementById('overlay-text');
            if (!overlay || !textEl) return;
            if (payload.loading) {{
                const dots = document.createElement('span');
                dots.className = 'loading-dots';
                dots.setAttribute('aria-hidden', 'true');
                for (let index = 0; index < 3; index += 1) {{
                    const dot = document.createElement('span');
                    dot.className = 'loading-dot';
                    dots.appendChild(dot);
                }}
                textEl.replaceChildren(dots);
            }} else {{
                textEl.textContent = String(payload.text ?? '');
            }}
            textEl.classList.toggle('loading', Boolean(payload.loading));
            overlay.style.left = '0px';
            overlay.style.top = '0px';
            overlay.style.display = 'inline-block';
            overlay.classList.add('visible');
            window.__quicktranslateOpen = true;
        }})();"#
    );
    let _ = overlay.eval(&script);

    if OVERLAY_DISMISSED.load(Ordering::SeqCst) {
        return Ok(());
    }

    if let Ok((cursor_x, cursor_y)) = screen::get_cursor_position() {
        let _ = overlay.set_position(Position::Physical(PhysicalPosition::new(
            cursor_x + 18,
            cursor_y + 18,
        )));
    }
    overlay.show().map_err(|err| format!("Failed to show overlay: {err}"))?;

    Ok(())
}

#[cfg(test)]
mod overlay_tests {
    use super::overlay_window_size;

    #[test]
    fn translated_text_window_grows_for_wrapped_lines() {
        let (_, one_line_height) = overlay_window_size("agent", false);
        let (_, multiple_lines_height) = overlay_window_size(
            "這是一段很長的翻譯文字，需要換行顯示，不應該被固定高度裁掉",
            false,
        );

        assert!(multiple_lines_height > one_line_height);
    }

    #[test]
    fn loading_overlay_remains_compact() {
        assert_eq!(overlay_window_size("", true), (44.0, 32.0));
    }
}

fn start_overlay_tracking(app: tauri::AppHandle) {
    if CURSOR_FOLLOWER_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    std::thread::spawn(move || {
        let mut mouse_buttons_were_down = [1, 2, 3, 4, 5].map(screen::is_mouse_button_down);

        loop {
            let Some(window) = app.get_webview_window("translation-overlay") else {
                std::thread::sleep(Duration::from_millis(16));
                continue;
            };

            let left_button_is_down = screen::is_mouse_button_down(1);
            if left_button_is_down && !mouse_buttons_were_down[0] {
                OVERLAY_DISMISSED.store(true, Ordering::SeqCst);
                let _ = window.hide();
            }
            let configured_mouse_trigger = MOUSE_SIDE_TRIGGER.load(Ordering::SeqCst);
            for button in 1..=5 {
                let button_is_down = screen::is_mouse_button_down(button);
                if button_is_down && !mouse_buttons_were_down[(button - 1) as usize] {
                    if button == configured_mouse_trigger {
                        let app_for_trigger = app.clone();
                        std::thread::spawn(move || handle_translation_shortcut(app_for_trigger));
                    }
                }
                mouse_buttons_were_down[(button - 1) as usize] = button_is_down;
            }

            if window.is_visible().unwrap_or(false) {
                if let Ok((cursor_x, cursor_y)) = screen::get_cursor_position() {
                    let _ = window.set_position(Position::Physical(PhysicalPosition::new(cursor_x + 18, cursor_y + 18)));
                }
            }

            std::thread::sleep(Duration::from_millis(8));
        }
    });
}

#[tauri::command]
fn translate_text_command(
    text: String,
    source_language: String,
    target_language: String,
) -> Result<serde_json::Value, String> {
    let result = translation::translate_text(&text, &source_language, &target_language)?;

    Ok(json!({
        "original": result.original,
        "translated": result.translated,
        "sourceLanguage": result.source_language,
        "targetLanguage": result.target_language,
    }))
}

fn handle_translation_shortcut(app: tauri::AppHandle) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let _serializer_guard = HOTKEY_SERIALIZER.lock();
    let delta = now.saturating_sub(LAST_HOTKEY_TS.load(Ordering::SeqCst));

    if HOTKEY_DEBOUNCE.swap(true, Ordering::SeqCst) || delta < HOTKEY_COOLDOWN_MS {
        HOTKEY_DEBOUNCE.store(false, Ordering::SeqCst);
        return;
    }

    if CAPTURE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        HOTKEY_DEBOUNCE.store(false, Ordering::SeqCst);
        return;
    }

    LAST_HOTKEY_TS.store(now, Ordering::SeqCst);
    OVERLAY_DISMISSED.store(false, Ordering::SeqCst);
    let settings = config::Settings::load().unwrap_or_else(|_| config::Settings::default());
    let interface_language = settings.interface_language.clone();

    show_overlay_status(&app, &interface_language, OverlayStatus::Translating);

    let _capture_lock = SCREEN_CAPTURE_LOCK.lock();

    let result = quick_capture();
    if let Err(err) = result {
        eprintln!("[QuickTranslate] capture failed: {err}");
        show_overlay_status(&app, &interface_language, OverlayStatus::CaptureFailed);
        CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
        HOTKEY_DEBOUNCE.store(false, Ordering::SeqCst);
        return;
    }

    let result = result.unwrap();
    let Some(text) = result.get("selectedText").and_then(|value| value.as_str()) else {
        CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
        HOTKEY_DEBOUNCE.store(false, Ordering::SeqCst);
        return;
    };

    if text.trim().is_empty() {
        show_overlay_status(&app, &interface_language, OverlayStatus::NoText);
        CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
        HOTKEY_DEBOUNCE.store(false, Ordering::SeqCst);
        return;
    }

    let translation = match translation::translate_text(
        text,
        &settings.source_language,
        &settings.target_language,
    ) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("[QuickTranslate] translation failed: {err}");
            show_overlay_status(&app, &interface_language, OverlayStatus::TranslationFailed);
            CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
            HOTKEY_DEBOUNCE.store(false, Ordering::SeqCst);
            return;
        }
    };

    let app_for_overlay = app.clone();
    let text_for_overlay = translation.translated;
    let _ = app.run_on_main_thread(move || {
        let _ = show_translation_overlay(app_for_overlay, text_for_overlay);
    });

    CAPTURE_IN_PROGRESS.store(false, Ordering::SeqCst);
    HOTKEY_DEBOUNCE.store(false, Ordering::SeqCst);
}

fn ensure_settings_window(app: &tauri::AppHandle) -> Result<(), String> {
    let language = config::Settings::load()
        .map(|settings| settings.interface_language)
        .unwrap_or_else(|_| "zh-Hant".to_string());
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.set_title(settings_window_title(&language));
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let settings_window = WebviewWindowBuilder::new(
        app,
        "settings",
        WebviewUrl::App("index.html".into()),
    )
    .title(settings_window_title(&language))
    .center()
    .resizable(true)
    .inner_size(820.0, 640.0)
    .min_inner_size(640.0, 480.0)
    .build()
    .map_err(|err| format!("Failed to create settings window: {err}"))?;

    let window_for_close = settings_window.clone();
    settings_window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window_for_close.hide();
        }
    });

    settings_window.show().map_err(|err| format!("Failed to show settings window: {err}"))?;
    settings_window.set_focus().map_err(|err| format!("Failed to focus settings window: {err}"))?;
    Ok(())
}

fn find_ollama_binary() -> Option<std::path::PathBuf> {
    let candidates = [
        std::env::var("OLLAMA_BIN").ok().map(std::path::PathBuf::from),
        Some(std::path::PathBuf::from(r"C:\Program Files\Ollama\ollama.exe")),
        Some(std::path::PathBuf::from(r"C:\Program Files (x86)\Ollama\ollama.exe")),
        Some(std::path::PathBuf::from(r"%USERPROFILE%\AppData\Local\Programs\Ollama\ollama.exe")),
    ];

    for candidate in candidates.into_iter().flatten() {
        if candidate.exists() {
            return Some(candidate);
        }
    }

    let path = std::env::var("PATH").unwrap_or_default();
    for entry in path.split(';') {
        let p = std::path::PathBuf::from(entry).join("ollama.exe");
        if p.exists() {
            return Some(p);
        }
    }

    None
}

fn ensure_local_model() -> Result<(), String> {
    let ollama = find_ollama_binary().ok_or_else(|| {
        let _ = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "Start-Process 'https://ollama.com/download/windows'",
            ])
            .status();
        "Ollama is not installed. The installer has been opened. Please finish installing it, then restart QuickTranslate.".to_string()
    })?;

    let list = Command::new(&ollama)
        .arg("list")
        .output()
        .map_err(|err| format!("Failed to query installed models: {err}"))?;

    let output = String::from_utf8_lossy(&list.stdout);
    let desired_model = "qwen2.5:3b";
    if output.contains(desired_model) {
        return Ok(());
    }

    let pull = Command::new(&ollama)
        .args(["pull", desired_model])
        .status()
        .map_err(|err| format!("Failed to download model: {err}"))?;

    if !pull.success() {
        return Err(format!("Ollama could not download the model '{desired_model}'."));
    }

    Ok(())
}

fn setup_tray(app: &tauri::AppHandle) -> Result<(), tauri::Error> {
    let language = config::Settings::load()
        .map(|settings| settings.interface_language)
        .unwrap_or_else(|_| "zh-Hant".to_string());
    let (settings_label, toggle_label, quit_label) = tray_labels(&language);
    let menu = Menu::new(app)?;
    let settings = MenuItem::with_id(app, "settings", settings_label, true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", toggle_label, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;

    menu.append(&settings)?;
    menu.append(&toggle)?;
    menu.append(&quit)?;

    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "settings" => {
                if let Err(err) = ensure_settings_window(app) {
                    eprintln!("{err}");
                }
            }
            "toggle" => {
                println!("Toggle translation requested");
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    app.manage(TrayMenuItems {
        settings,
        toggle,
        quit,
    });

    let _ = _tray;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    println!("QuickTranslate v{APP_VERSION} starting...");

    let _instance_guard = match SingleInstanceGuard::acquire() {
        Ok(guard) => guard,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(0);
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            setup_tray(app.handle())?;
            let _ = ensure_overlay_window(app.handle());
            start_overlay_tracking(app.handle().clone());

            let _ = std::thread::spawn(|| {
                if let Err(err) = ensure_local_model() {
                    eprintln!("{err}");
                }
            });

            let handle = app.handle().clone();
            let shortcut = config::Settings::load()
                .map(|settings| settings.shortcut)
                .unwrap_or_else(|_| "Ctrl+Shift+T".to_string());

            if let Err(err) = register_translation_shortcut(&handle, &shortcut, Some("Ctrl+Shift+T")) {
                eprintln!("{err}");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_settings,
            save_settings,
            ping,
            quick_capture,
            translate_text_command,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}