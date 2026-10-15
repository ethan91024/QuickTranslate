#[allow(dead_code)]
pub fn clamp_region(x: i32, y: i32, width: i32, height: i32, screen_width: i32, screen_height: i32) -> (i32, i32, i32, i32) {
    let max_x = (screen_width - width).max(0);
    let max_y = (screen_height - height).max(0);
    let x = x.clamp(0, max_x);
    let y = y.clamp(0, max_y);
    (x, y, width, height)
}

pub fn get_cursor_position() -> Result<(i32, i32), String> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT { x: 0, y: 0 };
    let success = unsafe { GetCursorPos(&mut point) };

    match success {
        Ok(_) => Ok((point.x, point.y)),
        Err(_) => Err("Unable to read the current mouse position.".to_string()),
    }
}

pub fn is_mouse_button_down(button: u8) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON, VK_XBUTTON1, VK_XBUTTON2,
    };

    let key = match button {
        1 => VK_LBUTTON,
        2 => VK_MBUTTON,
        3 => VK_RBUTTON,
        4 => VK_XBUTTON1,
        5 => VK_XBUTTON2,
        _ => return false,
    };

    unsafe { GetAsyncKeyState(key.0 as i32) < 0 }
}

pub fn capture_primary_screen() -> Result<image::RgbaImage, String> {
    use std::mem;
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, DIB_RGB_COLORS,
        GetDIBits, GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, RGBQUAD,
        SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    let width = unsafe { GetSystemMetrics(SM_CXSCREEN) } as u32;
    let height = unsafe { GetSystemMetrics(SM_CYSCREEN) } as u32;

    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err("Failed to get desktop DC.".to_string());
        }

        let memory_dc = CreateCompatibleDC(screen_dc);
        if memory_dc.is_invalid() {
            let _ = ReleaseDC(None, screen_dc);
            return Err("Failed to create memory DC.".to_string());
        }

        let bitmap = CreateCompatibleBitmap(screen_dc, width as i32, height as i32);
        if bitmap.is_invalid() {
            let _ = DeleteDC(memory_dc);
            let _ = ReleaseDC(None, screen_dc);
            return Err("Failed to create compatible bitmap.".to_string());
        }

        let previous_bitmap = SelectObject(memory_dc, bitmap);
        if let Err(err) = BitBlt(
            memory_dc,
            0,
            0,
            width as i32,
            height as i32,
            screen_dc,
            0,
            0,
            SRCCOPY,
        ) {
            let _ = SelectObject(memory_dc, previous_bitmap);
            let _ = DeleteObject(bitmap);
            let _ = DeleteDC(memory_dc);
            let _ = ReleaseDC(None, screen_dc);
            return Err(format!("Failed to copy screen bitmap via BitBlt: {err}"));
        }

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD::default(); 1],
        };

        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let copied = GetDIBits(
            memory_dc,
            bitmap,
            0,
            height,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        let _ = SelectObject(memory_dc, previous_bitmap);
        let _ = DeleteObject(bitmap);
        let _ = DeleteDC(memory_dc);
        let _ = ReleaseDC(None, screen_dc);

        if copied == 0 {
            return Err("Failed to read bitmap bits from desktop capture.".to_string());
        }

        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }

        image::RgbaImage::from_raw(width, height, pixels)
            .ok_or_else(|| "Failed to construct captured screen image.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_region_inside_screen_bounds() {
        let (x, y, w, h) = clamp_region(1900, 950, 150, 150, 1920, 1080);
        assert_eq!((x, y, w, h), (1770, 930, 150, 150));
    }

    #[test]
    fn keeps_region_when_inside_bounds() {
        let (x, y, w, h) = clamp_region(100, 100, 200, 80, 1920, 1080);
        assert_eq!((x, y, w, h), (100, 100, 200, 80));
    }
}
