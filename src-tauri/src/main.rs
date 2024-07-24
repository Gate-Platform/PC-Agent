// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use tauri::{ Manager, SystemTray, Window, WindowBuilder, WindowUrl };
use tauri::{ CustomMenuItem, SystemTrayMenu, SystemTrayEvent, Position };
use global_hotkey::{
    hotkey::{ Code, HotKey, Modifiers },
    GlobalHotKeyEvent,
    GlobalHotKeyManager,
    HotKeyState,
};
use std::time::Duration;
use std::env;
mod context;
use context::screen::get_screen;
use context::audio::AudioManager;
use single_instance::SingleInstance;
use std::sync::Mutex as SyncMutex;
use auto_launch::*;
use anyhow::Result;
use serde::{ Deserialize, Serialize };
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug)]
struct Settings {
    groq_api_key: String,
    screen_context: bool,
    audio_context: bool,
}
pub const SETTINGS_FILE_PATH: &str = "./settings.json";

// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
#[tauri::command]
fn get_settings() -> Result<Settings, String> {
    let file_path = PathBuf::from(SETTINGS_FILE_PATH);

    let default_settings = Settings {
        groq_api_key: "".to_string(),
        screen_context: true,
        audio_context: true,
    };

    if !file_path.exists() {
        let serialized = serde_json
            ::to_string(&default_settings)
            .map_err(|err| format!("Failed to serialize default settings: {}", err))?;
        fs
            ::write(file_path, serialized)
            .map_err(|err| format!("Failed to write default settings file: {}", err))?;
        Ok(default_settings)
    } else {
        // Read the file
        let contents = fs
            ::read_to_string(file_path)
            .map_err(|err| format!("Failed to read settings file: {}", err))?;

        // Deserialize the JSON string into our Settings struct
        let settings: Settings = serde_json
            ::from_str(&contents)
            .map_err(|err| format!("Failed to deserialize settings: {}", err))?;

        Ok(settings)
    }
}

#[tauri::command]
fn new_chat() -> Result<(), String> {
    let rt = tokio::runtime::Runtime
        ::new()
        .map_err(|err| format!("Failed to create runtime: {}", err))?;
    if
        let Some(manager) = AudioManager::get_instance()
            .lock()
            .map_err(|err| format!("Failed to lock mutex: {}", err))?
            .as_ref()
    {
        rt.block_on(manager.reset_transcript());
    }
    Ok(())
}

// Function to update settings in local storage
#[tauri::command]
fn update_settings(settings: Settings) -> Result<(), String> {
    // let rt = tokio::runtime::Runtime
    //     ::new()
    //     .map_err(|err| format!("Failed to create runtime: {}", err))?;

    if
        let Some(manager) = AudioManager::get_instance()
            .lock()
            .map_err(|err| format!("Failed to lock mutex: {}", err))?
            .as_ref()
    {
        manager.set_enabled(settings.audio_context);
    }
    let file_path = PathBuf::from(SETTINGS_FILE_PATH);

    // Serialize the Settings struct into a JSON string
    let serialized = serde_json
        ::to_string(&settings)
        .map_err(|err| format!("Failed to serialize settings: {}", err))?;

    // Write the JSON string to the file
    fs
        ::write(file_path, serialized)
        .map_err(|err| format!("Failed to write settings file: {}", err))?;

    Ok(())
}

#[macro_use]
extern crate lazy_static;
lazy_static! {
    static ref AUDIO_MANAGER: SyncMutex<Option<AudioManager>> = SyncMutex::new(None);
}
impl AudioManager {
    pub fn init(model_path: &str, max_chars: usize) {
        let mut manager = AUDIO_MANAGER.lock().unwrap();
        *manager = Some(AudioManager::new(model_path, max_chars).unwrap());
        manager.as_ref().unwrap().set_enabled(get_settings().unwrap().audio_context)
    }

    pub fn get_instance() -> &'static SyncMutex<Option<AudioManager>> {
        &AUDIO_MANAGER
    }
}

#[derive(Serialize, Deserialize)]
struct AIContext {
    content: String,
    api_key: String,
}

#[tauri::command]
fn get_context() -> Result<AIContext, String> {
    let max_screen_chars = 4000; // 1000 tokens~

    let mut context = String::new();
    let settings = get_settings()?;
    println!("{:?}", settings);
    if !settings.screen_context && !settings.audio_context {
        return Ok(AIContext {
            content: context,
            api_key: settings.groq_api_key,
        });
    }
    context.push_str("PC CONTEXT\n");

    if settings.screen_context {
        let screen_context = get_screen(max_screen_chars).map_err(|err|
            format!("Failed to get screen: {}", err)
        )?;
        if !screen_context.is_empty() {
            context.push_str("SCREEN:\n");
            context.push_str(&screen_context);
        }
    }
    if settings.audio_context {
        let rt = tokio::runtime::Runtime
            ::new()
            .map_err(|err| format!("Failed to create runtime: {}", err))?;

        if
            let Some(manager) = AudioManager::get_instance()
                .lock()
                .map_err(|err| format!("Failed to lock mutex: {}", err))?
                .as_ref()
        {
            // Block on the async operation using the Tokio runtime
            let audio_context = rt.block_on(manager.get_full_transcription());
            if !audio_context.is_empty() {
                context.push_str("AUDIO:\n");
                context.push_str(&audio_context);
            }
        }
    }
    Ok(AIContext {
        content: context,
        api_key: settings.groq_api_key,
    })

    //format!("PC Context\nscreen:\n{}\naudio:\n{}", screen_context, audio_context)
}

#[tauri::command]
fn toggle_settings_window(app_handle: tauri::AppHandle) {
    let settings_window = app_handle
        .get_window("settings")
        .unwrap_or_else(|| panic!("Settings window not found"));

    if settings_window.is_visible().unwrap_or(false) {
        settings_window.hide().unwrap();
    } else {
        settings_window.show().unwrap();
        settings_window.set_focus().unwrap();
    }
}

fn main() {
    let max_audio_chars = 2000; // 500 tokens~
    AudioManager::init("./assets/ggml-tiny-q5_1.bin", max_audio_chars);
    if let Some(manager) = AudioManager::get_instance().lock().unwrap().as_ref() {
        manager.start_audio_capture().unwrap();
    }
    let auto = AutoLaunchBuilder::new()
        .set_app_name("pc-agent")
        .set_app_path(env::current_dir().unwrap().to_str().unwrap())
        .set_use_launch_agent(true)
        .build()
        .unwrap();

    if !auto.is_enabled().unwrap() {
        auto.enable().unwrap();
    }

    let instance = SingleInstance::new("pc-assistant").unwrap();
    println!("instance a is single: {}", instance.is_single());

    if !instance.is_single() {
        std::process::exit(0);
    }

    let hotkey_manager = GlobalHotKeyManager::new().unwrap();
    let hotkey = HotKey::new(Some(Modifiers::ALT), Code::KeyQ);
    hotkey_manager.register(hotkey).unwrap();
    let receiver = GlobalHotKeyEvent::receiver();

    let quit = CustomMenuItem::new("quit".to_string(), "Quit");
    let mut name = CustomMenuItem::new("personal-computer-agent".to_string(), "PC Agent");
    name.enabled = false;

    let tray_menu = SystemTrayMenu::new().add_item(name).add_item(quit);
    tauri::Builder
        ::default()
        .setup(move |app| {
            let window = app.get_window("main").unwrap();

            window.set_skip_taskbar(true).unwrap();

            // Get the primary monitor
            let monitor = window.current_monitor().unwrap().unwrap();
            let screen_size = monitor.size();

            // Calculate window size (35% width, 100% height)
            let window_width = ((screen_size.width as f64) * 0.35) as u32;
            let window_height = screen_size.height;

            // Set window size
            window
                .set_size(
                    tauri::Size::Physical(tauri::PhysicalSize {
                        width: window_width,
                        height: window_height,
                    })
                )
                .unwrap();
            let app_handle = app.app_handle();
            create_settings_window(&app_handle);

            let app_handle_clone = app_handle.clone();
            std::thread::spawn(move || {
                loop {
                    if let Ok(event) = receiver.try_recv() {
                        if event.state == HotKeyState::Released {
                            let window = app_handle_clone.get_window("main").unwrap();
                            let settings_window = app_handle_clone.get_window("settings").unwrap();

                            if window.is_visible().unwrap() && window.is_focused().unwrap() {
                                window.hide().unwrap();
                                settings_window.hide().unwrap();
                            } else {
                                window.show().unwrap();
                                window.set_focus().unwrap();
                            }
                        }
                    }

                    std::thread::sleep(Duration::from_millis(100));
                }
            });
            Ok(())
        })

        .system_tray(SystemTray::new().with_menu(tray_menu))
        .on_system_tray_event(|app, event| {
            match event {
                SystemTrayEvent::LeftClick { position: _, size: _, .. } => {
                    let window = app.get_window("main").unwrap();

                    window.show().unwrap();
                    window.set_focus().unwrap();
                }
                SystemTrayEvent::MenuItemClick { id, .. } => {
                    match id.as_str() {
                        "quit" => {
                            std::process::exit(0);
                        }

                        _ => {}
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(
            tauri::generate_handler![
                toggle_settings_window,
                get_settings,
                update_settings,
                get_context,
                new_chat
            ]
        )
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            match event {
                tauri::RunEvent::ExitRequested { api, .. } => {
                    api.prevent_exit();
                }
                _ => {}
            }
        });
}
fn create_settings_window(app_handle: &tauri::AppHandle) {
    let settings_window = app_handle.get_window("settings");

    if let Some(window) = settings_window {
        window.show().unwrap();
        window.set_focus().unwrap();
    } else {
        let main_window = app_handle.get_window("main").unwrap();
        let main_position = main_window.outer_position().unwrap();
        let main_size = main_window.outer_size().unwrap();

        let settings_window = WindowBuilder::new(
            app_handle,
            "settings",
            WindowUrl::App("settings.html".into())
        )
            .title("")
            .decorations(false)
            .transparent(true)
            // .always_on_top(true)
            .skip_taskbar(true)
            .inner_size(300.0, 400.0)
            .position(
                (main_position.x as f64) + (main_size.width as f64) + 10.0,
                main_position.y as f64
            )
            .build()
            .unwrap();

        settings_window.hide().unwrap();
    }
}
