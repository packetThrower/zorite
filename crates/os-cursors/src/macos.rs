//! macOS backend — swizzle the `NSCursor` class factory methods.
//!
//! AppKit apps (gpui's `reset_cursor_rects` included) fetch cursors through
//! class methods — `+arrowCursor`, `+IBeamCursor`, … — every time they apply
//! one. `method_setImplementation` (supported objc runtime API) points each
//! of those at a shim that returns our cursor when one is installed and
//! falls through to the original otherwise. Everything in the process gets
//! themed; outside the window macOS draws its own cursors as usual.
//!
//! Two hard-won image rules (they cost a debugging session each):
//! `NSCursor` ignores `NSImage`'s size — the point size must ride an
//! `NSBitmapImageRep` via `setSize:` (a 64px rep at 20pt renders
//! Retina-crisp). And rep pixel data is *premultiplied* RGBA by default,
//! which matches XCursor's premultiplied BGRA after a channel swap.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;
use std::sync::{LazyLock, Mutex};

use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, Sel};
use objc2::{class, sel};
use objc2_app_kit::{NSBitmapImageRep, NSCursor, NSDeviceRGBColorSpace, NSImage};
use objc2_foundation::{NSPoint, NSSize};

use crate::{Cursor, Image, best_image};

unsafe extern "C" {
    fn class_getClassMethod(cls: *const AnyClass, name: Sel) -> *mut c_void;
    fn method_setImplementation(method: *mut c_void, imp: *mut c_void) -> *mut c_void;
}

/// Per-swizzled-selector state, keyed by [`sel_key`] (selectors are uniqued
/// by the runtime, so their name pointer is identity). `custom` is a
/// `Retained<NSCursor>` leaked into a raw pointer (0 = none installed);
/// `original` is the pre-swizzle IMP the shim falls back to.
struct Slot {
    custom: usize,
    original: usize,
}

static SLOTS: LazyLock<Mutex<HashMap<usize, Slot>>> = LazyLock::new(Default::default);

fn sel_key(sel: Sel) -> usize {
    sel.name().as_ptr() as usize
}

type Factory = unsafe extern "C" fn(*const AnyClass, Sel) -> *mut NSCursor;

unsafe extern "C" fn shim(cls: *const AnyClass, cmd: Sel) -> *mut NSCursor {
    let (custom, original) = {
        let slots = SLOTS.lock().unwrap();
        let slot = slots
            .get(&sel_key(cmd))
            .expect("shim dispatched for a selector that was never swizzled");
        (slot.custom, slot.original)
    };
    if custom != 0 {
        return custom as *mut NSCursor;
    }
    let original: Factory = unsafe { std::mem::transmute(original) };
    unsafe { original(cls, cmd) }
}

fn selector(cursor: Cursor) -> Sel {
    match cursor {
        Cursor::Arrow => sel!(arrowCursor),
        Cursor::IBeam => sel!(IBeamCursor),
        Cursor::Crosshair => sel!(crosshairCursor),
        Cursor::ClosedHand => sel!(closedHandCursor),
        Cursor::OpenHand => sel!(openHandCursor),
        Cursor::PointingHand => sel!(pointingHandCursor),
        Cursor::ResizeLeft => sel!(resizeLeftCursor),
        Cursor::ResizeRight => sel!(resizeRightCursor),
        Cursor::ResizeLeftRight => sel!(resizeLeftRightCursor),
        Cursor::ResizeUp => sel!(resizeUpCursor),
        Cursor::ResizeDown => sel!(resizeDownCursor),
        Cursor::ResizeUpDown => sel!(resizeUpDownCursor),
        // The two diagonal resizes are private AppKit selectors — the same
        // ones gpui itself uses.
        Cursor::ResizeUpLeftDownRight => sel!(_windowResizeNorthWestSouthEastCursor),
        Cursor::ResizeUpRightDownLeft => sel!(_windowResizeNorthEastSouthWestCursor),
        Cursor::IBeamVertical => sel!(IBeamCursorForVerticalLayout),
        Cursor::OperationNotAllowed => sel!(operationNotAllowedCursor),
        Cursor::DragLink => sel!(dragLinkCursor),
        Cursor::DragCopy => sel!(dragCopyCursor),
        Cursor::ContextualMenu => sel!(contextualMenuCursor),
    }
}

/// XCursor premultiplied BGRA → an `NSCursor` shown at `points` pt.
fn build(image: &Image, points: f32) -> Option<Retained<NSCursor>> {
    let (w, h) = (image.width as usize, image.height as usize);
    if image.bgra.len() < w * h * 4 {
        return None;
    }
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            ptr::null_mut(), // rep allocates its own buffer
            w as isize,
            h as isize,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            (w * 4) as isize,
            32,
        )?
    };
    let dst = rep.bitmapData();
    if dst.is_null() {
        return None;
    }
    for px in 0..w * h {
        let s = &image.bgra[px * 4..px * 4 + 4];
        // BGRA → RGBA; premultiplied in both, which is the rep's default.
        unsafe {
            *dst.add(px * 4) = s[2];
            *dst.add(px * 4 + 1) = s[1];
            *dst.add(px * 4 + 2) = s[0];
            *dst.add(px * 4 + 3) = s[3];
        }
    }
    let scale = points as f64 / w as f64;
    let size = NSSize::new(points as f64, h as f64 * scale);
    rep.setSize(size);
    let ns_image = NSImage::initWithSize(NSImage::alloc(), size);
    ns_image.addRepresentation(&rep);
    let hot = NSPoint::new(
        image.hotspot.0 as f64 * scale,
        image.hotspot.1 as f64 * scale,
    );
    Some(NSCursor::initWithImage_hotSpot(
        NSCursor::alloc(),
        &ns_image,
        hot,
    ))
}

pub(crate) fn install(cursor: Cursor, images: &[Image], points: f32) -> bool {
    // Prefer ≥3× the point size so the bitmap stays crisp on any display.
    let Some(image) = best_image(images, (points * 3.0).ceil() as u32) else {
        return false;
    };
    let Some(ns_cursor) = build(image, points) else {
        return false;
    };
    let sel = selector(cursor);
    let mut slots = SLOTS.lock().unwrap();
    let slot = match slots.entry(sel_key(sel)) {
        std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
        std::collections::hash_map::Entry::Vacant(e) => {
            let method = unsafe { class_getClassMethod(class!(NSCursor), sel) };
            if method.is_null() {
                return false;
            }
            let original = unsafe { method_setImplementation(method, shim as *mut c_void) };
            e.insert(Slot {
                custom: 0,
                original: original as usize,
            })
        }
    };
    if slot.custom != 0 {
        drop(unsafe { Retained::from_raw(slot.custom as *mut NSCursor) });
    }
    slot.custom = Retained::into_raw(ns_cursor) as usize;
    true
}

pub(crate) fn reset() {
    // The swizzles stay in place; with `custom` cleared the shim serves the
    // original IMPs, so the native cursors return.
    for slot in SLOTS.lock().unwrap().values_mut() {
        if slot.custom != 0 {
            drop(unsafe { Retained::from_raw(slot.custom as *mut NSCursor) });
            slot.custom = 0;
        }
    }
}
