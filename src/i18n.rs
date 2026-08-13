//! Localization glue. The `rust_i18n::i18n!` macro (invoked once in
//! `main.rs`) embeds the `locales/*.yml` catalogs into the binary at compile
//! time; the `t!()` macro reads the active locale at render time. This module
//! owns the small amount of process-global state that lives outside the
//! `AppView`: resolving the user's language choice (the persisted `"auto"` /
//! `"en"` / `"zh-CN"` id) to a concrete locale, and pushing it into
//! `rust-i18n`'s global so every window's next render picks it up.
//!
//! `t!` is used **only in the app**. The workspace crates stay host-agnostic:
//! they never call `t!` and receive no labels here. (TODO v0.11.0 Phase 4 -
//! injecting a `Labels` struct into the crates - is deferred; their strings
//! remain English for now.)

/// The offered Settings -> Language choices: `(persisted id, native-script
/// title)`. Language names render in their own script (so a zh-CN user can
/// find "English" and vice-versa); only the "Auto" entry is itself localized
/// at its call site.
pub const LANGUAGE_OPTS: &[(&str, &str)] =
    &[("auto", "Auto"), ("en", "English"), ("zh-CN", "简体中文")];

/// Resolve a persisted language choice to a concrete locale id.
///
/// - `"en"` / `"zh-CN"` -> themselves.
/// - `"auto"` (the default) -> the OS locale via `sys-locale`, mapped by a
///   `zh` prefix to `zh-CN` and everything else to `en`. Unknown persisted
///   values fall back to `en`, so a stale choice can never break rendering.
pub fn resolve_locale(choice: &str) -> &'static str {
    match choice {
        "en" => "en",
        "zh-CN" => "zh-CN",
        "auto" => {
            let loc = sys_locale::get_locale().unwrap_or_default();
            if loc.to_lowercase().starts_with("zh") {
                "zh-CN"
            } else {
                "en"
            }
        }
        _ => "en",
    }
}

/// Push the resolved locale into `rust-i18n`'s global so `t!` reads it. Called
/// at boot (after the persisted choice loads) and on every Settings change.
pub fn apply_locale(choice: &str) {
    rust_i18n::set_locale(resolve_locale(choice));
}

/// The localized labels injected into `gpui_editor`'s context menus and chrome
/// (right-click menu items, the code-card / math `Copy` chips, the table and
/// "Turn into" menus). The crate stays host-agnostic — it never calls `t!` —
/// so the app supplies these strings; re-inject on language switch.
pub fn editor_labels() -> gpui_editor::Labels {
    gpui_editor::Labels {
        cut: rust_i18n::t!("ctx.cut").into(),
        copy: rust_i18n::t!("ctx.copy").into(),
        copy_as_markdown: rust_i18n::t!("ctx.copy_as_markdown").into(),
        paste: rust_i18n::t!("ctx.paste").into(),
        code_copy: rust_i18n::t!("ctx.code_copy").into(),
        math_copy: rust_i18n::t!("ctx.math_copy").into(),
        turn_into: rust_i18n::t!("ctx.turn_into").into(),
        text: rust_i18n::t!("ctx.text").into(),
        heading_1: rust_i18n::t!("ctx.heading_1").into(),
        heading_2: rust_i18n::t!("ctx.heading_2").into(),
        heading_3: rust_i18n::t!("ctx.heading_3").into(),
        bulleted_list: rust_i18n::t!("ctx.bulleted_list").into(),
        numbered_list: rust_i18n::t!("ctx.numbered_list").into(),
        todo: rust_i18n::t!("ctx.todo").into(),
        quote: rust_i18n::t!("ctx.quote").into(),
        callout: rust_i18n::t!("ctx.callout").into(),
        code_block: rust_i18n::t!("ctx.code_block").into(),
        math_block: rust_i18n::t!("ctx.math_block").into(),
        insert_row_above: rust_i18n::t!("ctx.insert_row_above").into(),
        insert_row_below: rust_i18n::t!("ctx.insert_row_below").into(),
        duplicate_row: rust_i18n::t!("ctx.duplicate_row").into(),
        insert_column_left: rust_i18n::t!("ctx.insert_column_left").into(),
        insert_column_right: rust_i18n::t!("ctx.insert_column_right").into(),
        align_left: rust_i18n::t!("ctx.align_left").into(),
        align_center: rust_i18n::t!("ctx.align_center").into(),
        align_right: rust_i18n::t!("ctx.align_right").into(),
        grid_style: rust_i18n::t!("ctx.grid_style").into(),
        striped_style: rust_i18n::t!("ctx.striped_style").into(),
        header_style: rust_i18n::t!("ctx.header_style").into(),
        minimal_style: rust_i18n::t!("ctx.minimal_style").into(),
        delete_row: rust_i18n::t!("ctx.delete_row").into(),
        delete_column: rust_i18n::t!("ctx.delete_column").into(),
        delete_table: rust_i18n::t!("ctx.delete_table").into(),
        edit_properties: rust_i18n::t!("ctx.edit_properties").into(),
        delete_property: rust_i18n::t!("ctx.delete_property").into(),
        delete_image: rust_i18n::t!("ctx.delete_image").into(),
    }
}

/// The localized labels injected into `gpui_markdown`'s reader chrome (the
/// code-card `Copy` button). Same injection pattern as [`editor_labels`].
pub fn reader_labels() -> gpui_markdown::Labels {
    gpui_markdown::Labels {
        code_copy: rust_i18n::t!("ctx.code_copy").into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_choices_resolve_themselves() {
        assert_eq!(resolve_locale("en"), "en");
        assert_eq!(resolve_locale("zh-CN"), "zh-CN");
    }

    #[test]
    fn auto_resolves_to_an_offered_locale() {
        // No deterministic sys-locale mock; just assert it lands on a real one.
        let r = resolve_locale("auto");
        assert!(matches!(r, "en" | "zh-CN"));
    }

    #[test]
    fn unknown_choice_falls_back_to_english() {
        assert_eq!(resolve_locale("bogus"), "en");
        assert_eq!(resolve_locale(""), "en");
    }

    #[test]
    fn offered_opts_are_unique_and_stable() {
        let ids: Vec<&str> = LANGUAGE_OPTS.iter().map(|(id, _)| *id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "language ids must be unique");
        // The three the picker offers, in order.
        assert_eq!(ids, vec!["auto", "en", "zh-CN"]);
    }

    // Race-free catalog spot-checks: `t!(..., locale = ...)` reads a locale
    // WITHOUT mutating the process-global current locale, so these can't race
    // with the English-asserting tests in `dates.rs` (which rely on the `en`
    // fallback and never call `set_locale`).
    #[test]
    fn june_localizes_in_both_locales() {
        assert_eq!(rust_i18n::t!("dates.month.jun", locale = "en"), "June");
        assert_eq!(rust_i18n::t!("dates.month.jun", locale = "zh-CN"), "六月");
    }

    #[test]
    fn a_settings_and_slash_key_exist_in_both_locales() {
        // Any non-empty value proves the key is present in that catalog.
        assert!(!rust_i18n::t!("settings.nav.appearance", locale = "en").is_empty());
        assert!(!rust_i18n::t!("settings.nav.appearance", locale = "zh-CN").is_empty());
        assert!(!rust_i18n::t!("slash.cat.markdown", locale = "en").is_empty());
        assert!(!rust_i18n::t!("slash.cat.markdown", locale = "zh-CN").is_empty());
    }

    #[test]
    fn every_ctx_menu_key_exists_in_both_locales() {
        // The context-menu / chrome keys injected into the editor and reader
        // crates must resolve in both catalogs (a missing zh-CN entry would
        // silently fall back to English).
        macro_rules! ctx_key {
            ($key:literal) => {
                assert!(
                    !rust_i18n::t!($key, locale = "en").is_empty(),
                    "{} missing in en",
                    stringify!($key)
                );
                assert!(
                    !rust_i18n::t!($key, locale = "zh-CN").is_empty(),
                    "{} missing in zh-CN",
                    stringify!($key)
                );
            };
        }
        ctx_key!("ctx.cut");
        ctx_key!("ctx.copy");
        ctx_key!("ctx.copy_as_markdown");
        ctx_key!("ctx.paste");
        ctx_key!("ctx.code_copy");
        ctx_key!("ctx.math_copy");
        ctx_key!("ctx.turn_into");
        ctx_key!("ctx.text");
        ctx_key!("ctx.heading_1");
        ctx_key!("ctx.heading_2");
        ctx_key!("ctx.heading_3");
        ctx_key!("ctx.bulleted_list");
        ctx_key!("ctx.numbered_list");
        ctx_key!("ctx.todo");
        ctx_key!("ctx.quote");
        ctx_key!("ctx.callout");
        ctx_key!("ctx.code_block");
        ctx_key!("ctx.math_block");
        ctx_key!("ctx.insert_row_above");
        ctx_key!("ctx.insert_row_below");
        ctx_key!("ctx.duplicate_row");
        ctx_key!("ctx.insert_column_left");
        ctx_key!("ctx.insert_column_right");
        ctx_key!("ctx.align_left");
        ctx_key!("ctx.align_center");
        ctx_key!("ctx.align_right");
        ctx_key!("ctx.grid_style");
        ctx_key!("ctx.striped_style");
        ctx_key!("ctx.header_style");
        ctx_key!("ctx.minimal_style");
        ctx_key!("ctx.delete_row");
        ctx_key!("ctx.delete_column");
        ctx_key!("ctx.delete_table");
        ctx_key!("ctx.edit_properties");
        ctx_key!("ctx.delete_property");
        ctx_key!("ctx.delete_image");
    }
}
