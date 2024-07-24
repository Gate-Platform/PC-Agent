use win_screenshot::prelude::*;
use std::time::Instant;
use windows::Win32::Foundation::{ BOOL, HWND, LPARAM, RECT };
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows,
    GetClassNameW,
    GetWindowTextLengthW,
    GetWindowTextW,
    IsWindowVisible,
    GetClientRect,
    GetWindowThreadProcessId,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use std::fs;

use tokio::runtime::Runtime;
use anyhow::{ anyhow, Result, Error };
use image::{ DynamicImage, RgbaImage };
use tempfile::NamedTempFile;
use tokio::task;
use windows::{
    core::HSTRING,
    Globalization::Language,
    Graphics::Imaging::{ BitmapDecoder, SoftwareBitmap },
    Media::Ocr::OcrEngine,
    Storage::{ FileAccessMode, StorageFile },
};

use windows::core::HRESULT;

const E_ACCESSDENIED: HRESULT = HRESULT(0x80070005u32 as i32);
fn ocr(path: &str) -> windows::core::Result<String> {
    let bitmap = open_image_as_bitmap(path)?;
    let ocr_result = ocr_from_bitmap(bitmap)?;
    Ok(ocr_result)
}
fn open_image_as_bitmap(path: &str) -> windows::core::Result<SoftwareBitmap> {
    let path = fs::canonicalize(path);
    let path = match path {
        Ok(path) => path.to_string_lossy().replace("\\\\?\\", ""),
        Err(_) => {
            return Err(windows::core::Error::new(E_ACCESSDENIED, "Could not open file"));
        }
    };

    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))?.get()?;

    let bitmap = BitmapDecoder::CreateWithIdAsync(
        BitmapDecoder::PngDecoderId()?,
        &file.OpenAsync(FileAccessMode::Read)?.get()?
    )?.get()?;

    bitmap.GetSoftwareBitmapAsync()?.get()
}

fn ocr_from_bitmap(bitmap: SoftwareBitmap) -> windows::core::Result<String> {
    let lang = &OcrEngine::AvailableRecognizerLanguages()?.First()?.Current()?.LanguageTag()?;

    let lang = Language::CreateLanguage(lang)?;
    let engine = OcrEngine::TryCreateFromLanguage(&lang)?;

    let lines = engine.RecognizeAsync(&bitmap)?.get()?.Lines()?;
    let mut result = String::new();

    for line in lines {
        let line_text = line.Text()?.to_string_lossy();
        result.push_str(&line_text);
        result.push_str("\n");
    }
    Ok(result)
}

#[derive(Debug)]
pub struct HwndName {
    pub hwnd: isize,
    pub window_name: String,
}

#[derive(Debug)]
pub enum WLError {
    EnumWindowsError,
}

unsafe extern "system" fn wl_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let vec = lparam.0 as *mut Vec<HwndName>;
    if IsWindowVisible(hwnd) == false {
        return BOOL::from(true);
    }

    let gwtl = GetWindowTextLengthW(hwnd);
    if gwtl == 0 {
        return BOOL::from(true);
    }

    let max_count = 256; // Example maximum count, adjust based on your needs

    // Allocate memory for the class name buffer
    let mut class_name_buffer: Vec<u16> = vec![0; max_count as usize];

    // Pass the buffer as a mutable slice
    let class_name_length = GetClassNameW(hwnd, &mut class_name_buffer[..]);
    let class_name = String::from_utf16_lossy(&class_name_buffer[..class_name_length as usize])
        .trim_end_matches('\0')
        .to_string();
    let ignore_class_names = vec!["Progman", "TaskManagerWindow", "Windows.UI.Core.CoreWindow"];
    if ignore_class_names.contains(&class_name.as_str()) {
        return BOOL::from(true);
    }
    if class_name.contains("HwndWrapper") {
        return BOOL::from(true);
    }
    // println!("className: {}", class_name);

    let gwtl = GetWindowTextLengthW(hwnd);
    if gwtl == 0 {
        return BOOL::from(true);
    }

    let mut name_buf: Vec<u16> = vec![0; (gwtl + 1) as usize];

    let gwt = GetWindowTextW(hwnd, &mut name_buf);
    if gwt == 0 {
        return BOOL::from(true);
    }

    let name_buf = match name_buf.split_last() {
        Some((_, last)) => last,
        None => {
            return BOOL::from(true);
        }
    };

    let name = String::from_utf16_lossy(name_buf);

    let ignore_names = vec!["Settings"];

    if ignore_names.contains(&name.as_str()) {
        return BOOL::from(true);
    }
    if name.contains("settings.html") {
        return BOOL::from(true);
    }
    let mut rect = RECT::default(); // Use a zero-initialized RECT struct
    if GetClientRect(hwnd, &mut rect).is_err() {
        // println!("Failed to get client rectangle");
        return BOOL::from(true);
    }

    // Now you have the width and height in rect.right and rect.bottom
    let width = rect.right as isize;
    let height = rect.bottom as isize;

    // println!("window dimensions: {}x{}", width, height);
    if width * height < 10000 {
        // if too low pixels means window not rendered~
        return BOOL::from(true);
    }

    let current_process_id = GetCurrentProcessId();
    let mut process_id: u32 = 0;

    GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    if current_process_id == process_id {
        // ignore itself window
        return BOOL::from(true);
    }
    (*vec).push(HwndName {
        hwnd: hwnd.0 as isize,
        window_name: name,
    });

    BOOL::from(true)
}

pub fn get_window_list() -> Result<Vec<HwndName>, WLError> {
    let mut hwnd_name = Vec::new();
    unsafe {
        EnumWindows(
            Some(wl_callback),
            LPARAM(&mut hwnd_name as *mut Vec<HwndName> as isize)
        ).map_err(|_| WLError::EnumWindowsError)?;
    }

    Ok(hwnd_name)
}

#[derive(Debug, Clone)]
pub struct WindowContent {
    pub title: String,
    pub content: String,
}

async fn extract_text(window_info: HwndName) -> Option<WindowContent> {
    let hwnd = window_info.hwnd;
    println!("window: {}, hwnd: {}", window_info.window_name, hwnd);

    // Attempt to capture the window
    let buf = match capture_window(hwnd) {
        Ok(buf) => buf,
        Err(e) => {
            println!("Error capturing window: {}", e);
            return None;
        }
    };

    let img = DynamicImage::ImageRgba8(
        RgbaImage::from_raw(buf.width, buf.height, buf.pixels).unwrap()
    ).to_rgb8();
    let mut temp_file = match NamedTempFile::new() {
        Ok(file) => file,
        Err(e) => {
            println!("Failed to create temp file: {}", e);
            return None;
        }
    };

    // Save the image to the temporary file
    match img.write_to(&mut temp_file, image::ImageFormat::Png) {
        Ok(_) => (),
        Err(e) => {
            println!("Failed to write image to temp file: {}", e);
            return None;
        }
    }

    // Attempt to extract text from the bitmap
    match ocr(temp_file.path().display().to_string().as_str()) {
        Ok(text) =>
            Some(WindowContent {
                title: window_info.window_name,
                content: text,
            }),
        Err(e) => {
            println!("Error extracting text from bitmap: {}", e);
            None
        }
    }
}

async fn process_windows() -> Result<Vec<WindowContent>, Error> {
    let window_list = get_window_list().map_err(|e| {
        anyhow!("Failed to get window list: {:?}", e)
    })?;

    let mut tasks = Vec::with_capacity(window_list.len());

    // Spawn a task for each window and store the join handles in the tasks vector
    for window_info in window_list {
        let task = task::spawn(async move { extract_text(window_info).await });
        tasks.push(task);
    }

    // Await all futures concurrently
    let results = futures::future::try_join_all(tasks).await?;

    Ok(results.into_iter().flatten().collect())
}

pub fn get_screen(max_chars: usize) -> Result<String, Error> {
    let start_time = Instant::now();

    let rt = Runtime::new()?;
    let window_contents = rt.block_on(async { process_windows().await })?;

    let duration = start_time.elapsed();
    println!("Time taken: {:?}", duration);

    let mut combined_content = String::new();

    for window_content in window_contents.into_iter() {
        // Calculate the length of the next piece of content to be added
        let next_content = format!("{}:\n{}\n\n", window_content.title, window_content.content);

        // Check if adding this content would exceed the max chars limit
        if combined_content.len() + next_content.len() > max_chars {
            break; // Stop adding more content
        }

        // Add the content and update the total character count
        combined_content.push_str(&next_content);
    }

    Ok(combined_content)
}
