//! Windows backend — re-map standard cursors after `WM_SETCURSOR`.
//!
//! gpui (like most toolkits) answers `WM_SETCURSOR` with
//! `SetCursor(LoadCursorW(IDC_*))`. Standard cursor handles are process-wide
//! singletons, so a thread-scoped `WH_CALLWNDPROCRET` hook can watch each
//! `WM_SETCURSOR` complete, ask `GetCursor()` which standard cursor the
//! toolkit just set, and swap in ours. No subclassing, no toolkit changes.
//!
//! Granularity is the standard-handle set: Windows aliases both I-beams onto
//! `IDC_IBEAM`, all horizontal resizes onto `IDC_SIZEWE`, hands onto
//! `IDC_HAND`, and hand-offs like `ClosedHand` onto `IDC_ARROW` — only the
//! nine unambiguous cursors install; the aliased rest return `false` (their
//! primary's art shows). A direct `SetCursor` outside `WM_SETCURSOR` (a
//! style change without mouse movement) shows the native cursor until the
//! next mouse move re-sends `WM_SETCURSOR` — imperceptible in practice.
//!
//! NEEDS LIVE VERIFICATION on a Windows machine — compiles on CI, untested.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, Once};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CWPRETSTRUCT, CallNextHookEx, CreateIconIndirect, GetCursor, GetSystemMetrics, HCURSOR,
    ICONINFO, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_IBEAM, IDC_NO, IDC_SIZENESW, IDC_SIZENS,
    IDC_SIZENWSE, IDC_SIZEWE, LoadCursorW, PCWSTR, SM_CXCURSOR, SetCursor, SetWindowsHookExW,
    WH_CALLWNDPROCRET, WM_SETCURSOR,
};

use crate::{Cursor, Image, best_image};

/// standard HCURSOR value → our HCURSOR value.
static REMAP: LazyLock<Mutex<HashMap<usize, usize>>> = LazyLock::new(Default::default);
static HOOK: Once = Once::new();

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = unsafe { &*(lparam.0 as *const CWPRETSTRUCT) };
        if msg.message == WM_SETCURSOR {
            let current = unsafe { GetCursor() };
            let custom = REMAP.lock().unwrap().get(&(current.0 as usize)).copied();
            if let Some(custom) = custom {
                unsafe { SetCursor(Some(HCURSOR(custom as *mut _))) };
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// The standard cursor this logical cursor owns, or `None` when Windows
/// aliases it under another (see the module doc).
fn standard_id(cursor: Cursor) -> Option<PCWSTR> {
    match cursor {
        Cursor::Arrow => Some(IDC_ARROW),
        Cursor::IBeam => Some(IDC_IBEAM),
        Cursor::Crosshair => Some(IDC_CROSS),
        Cursor::PointingHand => Some(IDC_HAND),
        Cursor::ResizeLeftRight => Some(IDC_SIZEWE),
        Cursor::ResizeUpDown => Some(IDC_SIZENS),
        Cursor::ResizeUpLeftDownRight => Some(IDC_SIZENWSE),
        Cursor::ResizeUpRightDownLeft => Some(IDC_SIZENESW),
        Cursor::OperationNotAllowed => Some(IDC_NO),
        _ => None,
    }
}

/// Premultiplied BGRA → an alpha cursor via `CreateIconIndirect`.
fn build(image: &Image) -> Option<usize> {
    let (w, h) = (image.width as i32, image.height as i32);
    if image.bgra.len() < (w * h * 4) as usize {
        return None;
    }
    unsafe {
        let color = CreateBitmap(w, h, 1, 32, Some(image.bgra.as_ptr() as *const _));
        // The mask is required but unused for 32bpp alpha cursors.
        let mask_bits = vec![0u8; (w as usize).div_ceil(16) * 2 * h as usize];
        let mask = CreateBitmap(w, h, 1, 1, Some(mask_bits.as_ptr() as *const _));
        let info = ICONINFO {
            fIcon: false.into(),
            xHotspot: image.hotspot.0,
            yHotspot: image.hotspot.1,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon.ok().map(|i| i.0 as usize)
    }
}

pub(crate) fn install(cursor: Cursor, images: &[Image], _points: f32) -> bool {
    let Some(id) = standard_id(cursor) else {
        return false;
    };
    // Windows shows the bitmap at its pixel size — match the system cursor
    // size (DPI-aware). ponytail: no resampling if the pack lacks that size;
    // the nearest larger frame shows slightly big. Add scaling if it matters.
    let target = unsafe { GetSystemMetrics(SM_CXCURSOR) }.max(16) as u32;
    let Some(image) = best_image(images, target) else {
        return false;
    };
    let Some(custom) = build(image) else {
        return false;
    };
    let Ok(standard) = (unsafe { LoadCursorW(None, id) }) else {
        return false;
    };
    HOOK.call_once(|| unsafe {
        // Thread-scoped: `install` runs on the UI thread, so the hook lands
        // on the thread that owns the windows.
        let _ = SetWindowsHookExW(WH_CALLWNDPROCRET, Some(hook), None, GetCurrentThreadId());
    });
    // ponytail: a replaced custom cursor leaks its HICON — at most nine per
    // pack switch; DestroyIcon bookkeeping when packs switch live.
    REMAP.lock().unwrap().insert(standard.0 as usize, custom);
    true
}

pub(crate) fn reset() {
    REMAP.lock().unwrap().clear();
}
