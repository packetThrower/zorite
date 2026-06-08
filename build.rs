// Build script. Embeds the Windows .exe icon into the PE resource
// section, so Explorer / taskbar / Alt-Tab / Start menu show the Zorite
// icon on the .exe itself. cargo-packager's `icons` config covers the
// bundle-level metadata (installer branding, shortcut icon); this is the
// separate layer the PE carries.
//
// `embed_resource::compile` is a no-op on non-Windows targets, so this
// builds cleanly on macOS / Linux too.

fn main() {
    let _ = embed_resource::compile("resources/icon.rc", embed_resource::NONE);
}
