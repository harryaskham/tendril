use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayInfo {
    pub id: String,
    pub name: String,
    pub bounds: Bounds,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: String,
    pub title: String,
    pub app_name: Option<String>,
    pub process_id: u32,
    pub bounds: Bounds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierKey {
    Ctrl,
    Alt,
    Shift,
    Meta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

pub fn discover_displays() -> Result<Vec<DisplayInfo>, String> {
    imp::discover_displays()
}

pub fn discover_windows() -> Result<Vec<WindowInfo>, String> {
    imp::discover_windows()
}

pub fn capture_display_png(display_id: &str) -> Result<Vec<u8>, String> {
    imp::capture_display_png(display_id)
}

pub fn capture_window_png(window_id: &str) -> Result<Vec<u8>, String> {
    imp::capture_window_png(window_id)
}

pub fn focus_window(window_id: &str) -> Result<(), String> {
    imp::focus_window(window_id)
}

pub fn send_text(text: &str) -> Result<(), String> {
    imp::send_text(text)
}

pub fn tap_key(key: &str) -> Result<(), String> {
    imp::tap_key(key)
}

pub fn hold_modifier(modifier: ModifierKey) -> Result<(), String> {
    imp::hold_modifier(modifier)
}

pub fn release_modifier(modifier: ModifierKey) -> Result<(), String> {
    imp::release_modifier(modifier)
}

pub fn click_mouse(button: MouseButton, x: i32, y: i32) -> Result<(), String> {
    imp::click_mouse(button, x, y)
}

pub fn drag_mouse(
    button: MouseButton,
    start_x: i32,
    start_y: i32,
    end_x: i32,
    end_y: i32,
) -> Result<(), String> {
    imp::drag_mouse(button, start_x, start_y, end_x, end_y)
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use super::{DisplayInfo, ModifierKey, MouseButton, WindowInfo};

    const MESSAGE: &str =
        "native Windows runtime bindings are only available when building for Windows";

    pub fn discover_displays() -> Result<Vec<DisplayInfo>, String> {
        Err(MESSAGE.to_owned())
    }

    pub fn discover_windows() -> Result<Vec<WindowInfo>, String> {
        Err(MESSAGE.to_owned())
    }

    pub fn capture_display_png(_display_id: &str) -> Result<Vec<u8>, String> {
        Err(MESSAGE.to_owned())
    }

    pub fn capture_window_png(_window_id: &str) -> Result<Vec<u8>, String> {
        Err(MESSAGE.to_owned())
    }

    pub fn focus_window(_window_id: &str) -> Result<(), String> {
        Err(MESSAGE.to_owned())
    }

    pub fn send_text(_text: &str) -> Result<(), String> {
        Err(MESSAGE.to_owned())
    }

    pub fn tap_key(_key: &str) -> Result<(), String> {
        Err(MESSAGE.to_owned())
    }

    pub fn hold_modifier(_modifier: ModifierKey) -> Result<(), String> {
        Err(MESSAGE.to_owned())
    }

    pub fn release_modifier(_modifier: ModifierKey) -> Result<(), String> {
        Err(MESSAGE.to_owned())
    }

    pub fn click_mouse(_button: MouseButton, _x: i32, _y: i32) -> Result<(), String> {
        Err(MESSAGE.to_owned())
    }

    pub fn drag_mouse(
        _button: MouseButton,
        _start_x: i32,
        _start_y: i32,
        _end_x: i32,
        _end_y: i32,
    ) -> Result<(), String> {
        Err(MESSAGE.to_owned())
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::OsString;
    use std::io::Cursor;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStringExt;
    use std::path::Path;
    use std::ptr::{null, null_mut};
    use std::thread;
    use std::time::Duration;

    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
    use windows_sys::Win32::Foundation::{
        BOOL, CloseHandle, HANDLE, HBITMAP, HDC, HGDIOBJ, HMONITOR, HWND, LPARAM, RECT,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
        CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, GetMonitorInfoW,
        MONITORINFO, MONITORINFOEXW, SRCCOPY, SelectObject,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN,
        MOUSEEVENTF_RIGHTUP, MOUSEINPUT, MapVirtualKeyW, SendInput, SetCursorPos, VK_BACK,
        VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_LWIN, VK_MENU,
        VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP, VkKeyScanW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumDisplayMonitors, EnumWindows, GetDC, GetDesktopWindow, GetWindowRect,
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
        PrintWindow, ReleaseDC, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };

    use super::{Bounds, DisplayInfo, ModifierKey, MouseButton, WindowInfo};

    const PW_RENDERFULLCONTENT: u32 = 0x0000_0002;

    pub fn discover_displays() -> Result<Vec<DisplayInfo>, String> {
        let mut displays = Vec::<DisplayInfo>::new();
        let result = unsafe {
            EnumDisplayMonitors(
                0,
                null(),
                Some(enum_display_monitors),
                (&mut displays as *mut Vec<DisplayInfo>) as isize,
            )
        };
        if result == 0 {
            return Err("EnumDisplayMonitors failed".to_owned());
        }

        displays.sort_by(|left, right| {
            (left.bounds.y, left.bounds.x, left.name.to_ascii_lowercase()).cmp(&(
                right.bounds.y,
                right.bounds.x,
                right.name.to_ascii_lowercase(),
            ))
        });

        for (index, display) in displays.iter_mut().enumerate() {
            display.id = format!("{}", index + 1);
        }

        Ok(displays)
    }

    pub fn discover_windows() -> Result<Vec<WindowInfo>, String> {
        let mut windows = Vec::<WindowInfo>::new();
        let result = unsafe {
            EnumWindows(
                Some(enum_windows),
                (&mut windows as *mut Vec<WindowInfo>) as isize,
            )
        };
        if result == 0 {
            return Err("EnumWindows failed".to_owned());
        }

        windows.sort_by(|left, right| {
            (
                left.bounds.y,
                left.bounds.x,
                left.title.to_ascii_lowercase(),
                left.id.clone(),
            )
                .cmp(&(
                    right.bounds.y,
                    right.bounds.x,
                    right.title.to_ascii_lowercase(),
                    right.id.clone(),
                ))
        });
        Ok(windows)
    }

    pub fn capture_display_png(display_id: &str) -> Result<Vec<u8>, String> {
        let display = discover_displays()?
            .into_iter()
            .find(|display| display.id == display_id)
            .ok_or_else(|| format!("display target `{display_id}` was not found"))?;
        capture_rect_png(
            display.bounds.x,
            display.bounds.y,
            display.bounds.width,
            display.bounds.height,
        )
    }

    pub fn capture_window_png(window_id: &str) -> Result<Vec<u8>, String> {
        let hwnd = parse_window_id(window_id)?;
        let rect = get_window_rect(hwnd)?;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return Err(format!("window `{window_id}` has invalid bounds"));
        }
        capture_window_png_inner(hwnd, rect.left, rect.top, width as u32, height as u32)
    }

    pub fn focus_window(window_id: &str) -> Result<(), String> {
        let hwnd = parse_window_id(window_id)?;
        unsafe {
            ShowWindow(hwnd, SW_RESTORE);
            BringWindowToTop(hwnd);
            if SetForegroundWindow(hwnd) == 0 {
                return Err(format!("SetForegroundWindow failed for `{window_id}`"));
            }
        }
        Ok(())
    }

    pub fn send_text(text: &str) -> Result<(), String> {
        let mut inputs = Vec::with_capacity(text.encode_utf16().count() * 2);
        for unit in text.encode_utf16() {
            inputs.push(keyboard_input_unicode(unit, false));
            inputs.push(keyboard_input_unicode(unit, true));
        }
        send_inputs(&inputs)
    }

    pub fn tap_key(key: &str) -> Result<(), String> {
        if let Some(vk) = named_virtual_key(key) {
            tap_virtual_key(vk)
        } else {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => tap_character(ch),
                _ => Err(format!("unsupported Windows key `{key}`")),
            }
        }
    }

    pub fn hold_modifier(modifier: ModifierKey) -> Result<(), String> {
        press_virtual_key(modifier_virtual_key(modifier), false)
    }

    pub fn release_modifier(modifier: ModifierKey) -> Result<(), String> {
        press_virtual_key(modifier_virtual_key(modifier), true)
    }

    pub fn click_mouse(button: MouseButton, x: i32, y: i32) -> Result<(), String> {
        unsafe {
            if SetCursorPos(x, y) == 0 {
                return Err(format!("SetCursorPos failed for {x},{y}"));
            }
        }
        let (down, up) = mouse_event_flags(button);
        send_inputs(&[mouse_input(down), mouse_input(up)])
    }

    pub fn drag_mouse(
        button: MouseButton,
        start_x: i32,
        start_y: i32,
        end_x: i32,
        end_y: i32,
    ) -> Result<(), String> {
        unsafe {
            if SetCursorPos(start_x, start_y) == 0 {
                return Err(format!("SetCursorPos failed for {start_x},{start_y}"));
            }
        }
        let (down, up) = mouse_event_flags(button);
        send_inputs(&[mouse_input(down)])?;
        thread::sleep(Duration::from_millis(20));
        unsafe {
            if SetCursorPos(end_x, end_y) == 0 {
                return Err(format!("SetCursorPos failed for {end_x},{end_y}"));
            }
        }
        send_inputs(&[mouse_input(up)])
    }

    unsafe extern "system" fn enum_display_monitors(
        monitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let displays = &mut *(lparam as *mut Vec<DisplayInfo>);
        let mut info: MONITORINFOEXW = zeroed();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        let success = GetMonitorInfoW(
            monitor,
            (&mut info as *mut MONITORINFOEXW).cast::<MONITORINFO>(),
        );
        if success != 0 {
            let bounds = info.monitorInfo.rcMonitor;
            let width = bounds.right - bounds.left;
            let height = bounds.bottom - bounds.top;
            if width > 0 && height > 0 {
                displays.push(DisplayInfo {
                    id: String::new(),
                    name: utf16z_to_string(&info.szDevice),
                    bounds: Bounds {
                        x: bounds.left,
                        y: bounds.top,
                        width: width as u32,
                        height: height as u32,
                    },
                });
            }
        }
        1
    }

    unsafe extern "system" fn enum_windows(hwnd: HWND, lparam: LPARAM) -> BOOL {
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }

        let title_length = GetWindowTextLengthW(hwnd);
        if title_length <= 0 {
            return 1;
        }

        let mut title_buffer = vec![0_u16; title_length as usize + 1];
        let copied = GetWindowTextW(hwnd, title_buffer.as_mut_ptr(), title_buffer.len() as i32);
        if copied <= 0 {
            return 1;
        }
        let title = String::from_utf16_lossy(&title_buffer[..copied as usize])
            .trim()
            .to_owned();
        if title.is_empty() {
            return 1;
        }

        let Ok(rect) = get_window_rect(hwnd) else {
            return 1;
        };
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return 1;
        }

        let mut process_id = 0_u32;
        GetWindowThreadProcessId(hwnd, &mut process_id);
        if process_id == 0 {
            return 1;
        }

        let windows = &mut *(lparam as *mut Vec<WindowInfo>);
        windows.push(WindowInfo {
            id: format!("0x{:X}", hwnd as usize),
            title: title.clone(),
            app_name: process_name(process_id),
            process_id,
            bounds: Bounds {
                x: rect.left,
                y: rect.top,
                width: width as u32,
                height: height as u32,
            },
        });
        1
    }

    fn get_window_rect(hwnd: HWND) -> Result<Rect, String> {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let success = unsafe { GetWindowRect(hwnd, &mut rect) };
        if success == 0 {
            return Err(format!(
                "GetWindowRect failed for window 0x{:X}",
                hwnd as usize
            ));
        }
        Ok(Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        })
    }

    fn process_name(process_id: u32) -> Option<String> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
        if handle == 0 {
            return None;
        }
        let _guard = HandleGuard(handle);
        let mut buffer = vec![0_u16; 260];
        let mut length = buffer.len() as u32;
        let success =
            unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) };
        if success == 0 || length == 0 {
            return None;
        }
        let path = OsString::from_wide(&buffer[..length as usize]);
        let file_name = Path::new(&path)
            .file_stem()?
            .to_string_lossy()
            .trim()
            .to_owned();
        if file_name.is_empty() {
            None
        } else {
            Some(file_name)
        }
    }

    fn parse_window_id(window_id: &str) -> Result<HWND, String> {
        let trimmed = window_id.trim();
        let parsed = if let Some(hex) = trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"))
        {
            usize::from_str_radix(hex, 16)
        } else {
            trimmed.parse::<usize>()
        }
        .map_err(|error| format!("window target id `{window_id}` could not be parsed: {error}"))?;
        Ok(parsed as HWND)
    }

    fn capture_rect_png(x: i32, y: i32, width: u32, height: u32) -> Result<Vec<u8>, String> {
        let desktop = unsafe { GetDesktopWindow() };
        let screen_dc = unsafe { GetDC(desktop) };
        if screen_dc == 0 {
            return Err("GetDC failed for the desktop window".to_owned());
        }
        let screen_guard = DeviceContextGuard {
            hwnd: desktop,
            hdc: screen_dc,
        };
        let mem_dc = unsafe { CreateCompatibleDC(screen_dc) };
        if mem_dc == 0 {
            return Err("CreateCompatibleDC failed".to_owned());
        }
        let mem_guard = MemoryDeviceContextGuard(mem_dc);
        let bitmap = unsafe { CreateCompatibleBitmap(screen_dc, width as i32, height as i32) };
        if bitmap == 0 {
            return Err("CreateCompatibleBitmap failed".to_owned());
        }
        let bitmap_guard = BitmapGuard(bitmap);
        let selection = unsafe { SelectObject(mem_dc, bitmap as HGDIOBJ) };
        if selection == 0 {
            return Err("SelectObject failed".to_owned());
        }
        let selection_guard = SelectionGuard {
            hdc: mem_dc,
            previous: selection,
        };
        let copied = unsafe {
            BitBlt(
                mem_dc,
                0,
                0,
                width as i32,
                height as i32,
                screen_dc,
                x,
                y,
                SRCCOPY | CAPTUREBLT,
            )
        };
        if copied == 0 {
            return Err("BitBlt failed while capturing the display".to_owned());
        }

        let encoded = bitmap_to_png(screen_dc, bitmap, width, height);

        drop(selection_guard);
        drop(bitmap_guard);
        drop(mem_guard);
        drop(screen_guard);

        encoded
    }

    fn capture_window_png_inner(
        hwnd: HWND,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, String> {
        let desktop = unsafe { GetDesktopWindow() };
        let screen_dc = unsafe { GetDC(desktop) };
        if screen_dc == 0 {
            return Err("GetDC failed for the desktop window".to_owned());
        }
        let screen_guard = DeviceContextGuard {
            hwnd: desktop,
            hdc: screen_dc,
        };
        let mem_dc = unsafe { CreateCompatibleDC(screen_dc) };
        if mem_dc == 0 {
            return Err("CreateCompatibleDC failed".to_owned());
        }
        let mem_guard = MemoryDeviceContextGuard(mem_dc);
        let bitmap = unsafe { CreateCompatibleBitmap(screen_dc, width as i32, height as i32) };
        if bitmap == 0 {
            return Err("CreateCompatibleBitmap failed".to_owned());
        }
        let bitmap_guard = BitmapGuard(bitmap);
        let selection = unsafe { SelectObject(mem_dc, bitmap as HGDIOBJ) };
        if selection == 0 {
            return Err("SelectObject failed".to_owned());
        }
        let selection_guard = SelectionGuard {
            hdc: mem_dc,
            previous: selection,
        };

        let printed = unsafe { PrintWindow(hwnd, mem_dc, PW_RENDERFULLCONTENT) };
        if printed == 0 {
            let copied = unsafe {
                BitBlt(
                    mem_dc,
                    0,
                    0,
                    width as i32,
                    height as i32,
                    screen_dc,
                    x,
                    y,
                    SRCCOPY | CAPTUREBLT,
                )
            };
            if copied == 0 {
                return Err(
                    "PrintWindow and BitBlt both failed while capturing the window".to_owned(),
                );
            }
        }

        let encoded = bitmap_to_png(screen_dc, bitmap, width, height);

        drop(selection_guard);
        drop(bitmap_guard);
        drop(mem_guard);
        drop(screen_guard);

        encoded
    }

    fn bitmap_to_png(
        screen_dc: HDC,
        bitmap: HBITMAP,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, String> {
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [zeroed()],
        };
        let mut pixels = vec![0_u8; width as usize * height as usize * 4];
        let rows = unsafe {
            GetDIBits(
                screen_dc,
                bitmap,
                0,
                height,
                pixels.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS,
            )
        };
        if rows == 0 {
            return Err("GetDIBits failed".to_owned());
        }

        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            if pixel[3] == 0 {
                pixel[3] = 255;
            }
        }

        let image = ImageBuffer::<Rgba<u8>, _>::from_vec(width, height, pixels)
            .ok_or_else(|| "failed to build RGBA image buffer".to_owned())?;
        let mut encoded = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut encoded), ImageFormat::Png)
            .map_err(|error| format!("failed to encode PNG: {error}"))?;
        Ok(encoded)
    }

    fn tap_virtual_key(vk: u16) -> Result<(), String> {
        send_inputs(&[
            keyboard_input_virtual_key(vk, false),
            keyboard_input_virtual_key(vk, true),
        ])
    }

    fn tap_character(ch: char) -> Result<(), String> {
        let encoded: Vec<u16> = ch.to_string().encode_utf16().collect();
        if encoded.len() != 1 {
            return send_text(&ch.to_string());
        }
        let translated = unsafe { VkKeyScanW(encoded[0]) };
        if translated == -1 {
            return send_text(&ch.to_string());
        }
        let vk = (translated & 0xFF) as u16;
        let shift_state = ((translated >> 8) & 0xFF) as u8;
        let mut inputs = Vec::new();
        if shift_state & 1 != 0 {
            inputs.push(keyboard_input_virtual_key(VK_SHIFT as u16, false));
        }
        if shift_state & 2 != 0 {
            inputs.push(keyboard_input_virtual_key(VK_CONTROL as u16, false));
        }
        if shift_state & 4 != 0 {
            inputs.push(keyboard_input_virtual_key(VK_MENU as u16, false));
        }
        inputs.push(keyboard_input_virtual_key(vk, false));
        inputs.push(keyboard_input_virtual_key(vk, true));
        if shift_state & 4 != 0 {
            inputs.push(keyboard_input_virtual_key(VK_MENU as u16, true));
        }
        if shift_state & 2 != 0 {
            inputs.push(keyboard_input_virtual_key(VK_CONTROL as u16, true));
        }
        if shift_state & 1 != 0 {
            inputs.push(keyboard_input_virtual_key(VK_SHIFT as u16, true));
        }
        send_inputs(&inputs)
    }

    fn press_virtual_key(vk: u16, key_up: bool) -> Result<(), String> {
        send_inputs(&[keyboard_input_virtual_key(vk, key_up)])
    }

    fn named_virtual_key(key: &str) -> Option<u16> {
        match key.to_ascii_lowercase().as_str() {
            "enter" | "return" => Some(VK_RETURN as u16),
            "esc" | "escape" => Some(VK_ESCAPE as u16),
            "tab" => Some(VK_TAB as u16),
            "space" => Some(VK_SPACE as u16),
            "backspace" => Some(VK_BACK as u16),
            "delete" | "del" => Some(VK_DELETE as u16),
            "left" => Some(VK_LEFT as u16),
            "right" => Some(VK_RIGHT as u16),
            "up" => Some(VK_UP as u16),
            "down" => Some(VK_DOWN as u16),
            "home" => Some(VK_HOME as u16),
            "end" => Some(VK_END as u16),
            "pageup" => Some(VK_PRIOR as u16),
            "pagedown" => Some(VK_NEXT as u16),
            _ => None,
        }
    }

    fn modifier_virtual_key(modifier: ModifierKey) -> u16 {
        match modifier {
            ModifierKey::Ctrl => VK_CONTROL as u16,
            ModifierKey::Alt => VK_MENU as u16,
            ModifierKey::Shift => VK_SHIFT as u16,
            ModifierKey::Meta => VK_LWIN as u16,
        }
    }

    fn mouse_event_flags(button: MouseButton) -> (u32, u32) {
        match button {
            MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
            MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        }
    }

    fn send_inputs(inputs: &[INPUT]) -> Result<(), String> {
        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                size_of::<INPUT>() as i32,
            )
        };
        if sent != inputs.len() as u32 {
            Err(format!(
                "SendInput dispatched {sent} events out of {} requested",
                inputs.len()
            ))
        } else {
            Ok(())
        }
    }

    fn keyboard_input_unicode(unit: u16, key_up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: 0,
                    wScan: unit,
                    dwFlags: if key_up {
                        KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                    } else {
                        KEYEVENTF_UNICODE
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn keyboard_input_virtual_key(vk: u16, key_up: bool) -> INPUT {
        let scan_code = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16;
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: scan_code,
                    dwFlags: extended_key_flag(vk) | if key_up { KEYEVENTF_KEYUP } else { 0 },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn mouse_input(flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn extended_key_flag(vk: u16) -> u32 {
        match vk {
            value
                if value == VK_LEFT as u16
                    || value == VK_RIGHT as u16
                    || value == VK_UP as u16
                    || value == VK_DOWN as u16
                    || value == VK_HOME as u16
                    || value == VK_END as u16
                    || value == VK_PRIOR as u16
                    || value == VK_NEXT as u16
                    || value == VK_DELETE as u16
                    || value == VK_MENU as u16
                    || value == VK_CONTROL as u16
                    || value == VK_LWIN as u16 =>
            {
                KEYEVENTF_EXTENDEDKEY
            }
            _ => 0,
        }
    }

    fn utf16z_to_string(buffer: &[u16]) -> String {
        let end = buffer
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..end]).trim().to_owned()
    }

    #[derive(Debug, Clone, Copy)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    struct HandleGuard(HANDLE);

    impl Drop for HandleGuard {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    struct DeviceContextGuard {
        hwnd: HWND,
        hdc: HDC,
    }

    impl Drop for DeviceContextGuard {
        fn drop(&mut self) {
            unsafe {
                ReleaseDC(self.hwnd, self.hdc);
            }
        }
    }

    struct MemoryDeviceContextGuard(HDC);

    impl Drop for MemoryDeviceContextGuard {
        fn drop(&mut self) {
            unsafe {
                DeleteDC(self.0);
            }
        }
    }

    struct BitmapGuard(HBITMAP);

    impl Drop for BitmapGuard {
        fn drop(&mut self) {
            unsafe {
                DeleteObject(self.0 as HGDIOBJ);
            }
        }
    }

    struct SelectionGuard {
        hdc: HDC,
        previous: HGDIOBJ,
    }

    impl Drop for SelectionGuard {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.hdc, self.previous);
            }
        }
    }
}

impl fmt::Display for ModifierKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ctrl => "ctrl",
            Self::Alt => "alt",
            Self::Shift => "shift",
            Self::Meta => "meta",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ModifierKey, MouseButton};

    #[test]
    fn enums_remain_stable_for_cross_crate_callers() {
        assert_eq!(ModifierKey::Ctrl.to_string(), "ctrl");
        assert_eq!(format!("{:?}", MouseButton::Left), "Left");
    }
}
