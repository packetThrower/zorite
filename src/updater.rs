//! Boot-time update check against the project's GitHub Releases.
//!
//! Posture by design: **detection only**. We never download a replacement
//! bundle or rewrite anything on disk. The check discovers the latest tag,
//! compares it against `CARGO_PKG_VERSION`, and if a newer release exists
//! publishes an [`UpdateState`] global that the chrome reads to paint an amber
//! dot (the sidebar settings gear) and fill in Settings → Updates (version +
//! release-notes link). The user clicks through to the Releases page to install.
//!
//! Why no auto-install: macOS code-signing / notarization haven't shipped, and
//! the "swap the live binary on quit" dance is real cross-platform work.
//! Detection-only keeps the user in control of whether and when to update.
//!
//! The fetch is sync (`ureq`) on a `gpui::BackgroundExecutor` task. Offline →
//! the result is `None`, exactly as if no update existed (no error surfaced).

use std::time::Duration;

use serde::Deserialize;

/// Live state of the update check, installed as a gpui [`gpui::Global`] so any
/// render path can read it cheaply. `None` means "no check completed yet" OR
/// "we're already on the newest release" — render code treats both as "no dot".
#[derive(Debug, Clone, Default)]
pub struct UpdateState {
    /// `Some` when a newer release is available.
    pub available: Option<UpdateAvailable>,
}

impl gpui::Global for UpdateState {}

/// Available-update payload exposed to the chrome.
#[derive(Debug, Clone)]
pub struct UpdateAvailable {
    /// Bare version string (no leading `v`), e.g. `"0.2.0"` or `"0.2.0-beta.1"`.
    pub version: String,
    /// Browser URL for the GitHub Releases page entry — opened by the "View
    /// release" button in Settings → Updates.
    pub html_url: String,
    /// Release-notes body (Markdown); the full notes live at `html_url`.
    pub notes: String,
}

/// GitHub Releases API response shape — only the fields we use. `#[serde(default)]`
/// so a partial response doesn't fail the whole parse.
#[derive(Debug, Deserialize)]
struct Release {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// Spawn the update check on the background pool, then publish the result to the
/// [`UpdateState`] global and refresh open windows so the indicator repaints.
/// Used for both the boot check and Settings → Updates → "Check now".
pub fn spawn_check(include_prerelease: bool, cx: &mut gpui::App) {
    cx.spawn(async move |cx| {
        let result = cx
            .background_executor()
            .spawn(async move { check_for_update(env!("CARGO_PKG_VERSION"), include_prerelease) })
            .await;
        let available = match result {
            Ok(Some(a)) => a,
            Ok(None) => return,
            Err(err) => {
                log::info!("update check: semver: {err}");
                return;
            }
        };
        cx.update(|cx| {
            cx.set_global(UpdateState {
                available: Some(available),
            });
            cx.refresh_windows();
        });
    })
    .detach();
}

/// Query the project's GitHub Releases for the newest release the user's
/// `include_prerelease` preference allows.
///
/// `Ok(None)` when nothing is newer than `current_version`, the network call
/// fails, or the response doesn't parse. `Err` only if `current_version` isn't
/// valid semver (it comes from `CARGO_PKG_VERSION`, so never in practice).
///
/// Blocking — call from a `BackgroundExecutor` task, never the render thread.
pub fn check_for_update(
    current_version: &str,
    include_prerelease: bool,
) -> Result<Option<UpdateAvailable>, semver::Error> {
    let current = semver::Version::parse(current_version)?;
    let releases = match fetch_releases(include_prerelease) {
        Some(r) => r,
        None => return Ok(None),
    };
    let newest = releases
        .into_iter()
        // Drafts shouldn't surface via the public API, but filter defensively.
        .filter(|r| !r.draft)
        // Hide pre-releases unless the user opted in.
        .filter(|r| include_prerelease || !r.prerelease)
        // Parse each tag's semver (`vX.Y.Z[-pre]`); unparseable tags are skipped.
        .filter_map(|r| {
            let bare = r.tag_name.strip_prefix('v').unwrap_or(&r.tag_name);
            let ver = semver::Version::parse(bare).ok()?;
            Some((ver, r))
        })
        // Max by version, not API order: GitHub sorts by created_at, so a
        // backported patch on an older line could be newest-by-date.
        .max_by(|(a, _), (b, _)| a.cmp(b));
    let Some((newest_ver, release)) = newest else {
        return Ok(None);
    };
    if newest_ver <= current {
        return Ok(None);
    }
    Ok(Some(UpdateAvailable {
        version: newest_ver.to_string(),
        html_url: release.html_url,
        notes: release.body,
    }))
}

/// Hit the GitHub Releases API. `None` on any error — the caller treats that the
/// same as "no update", so the user never sees a "check failed" diagnostic
/// (logged at `info!`, since a transient offline state isn't a real problem).
fn fetch_releases(include_prerelease: bool) -> Option<Vec<Release>> {
    // Stable-only uses `/releases/latest` (single object); the pre-release path
    // needs the paginated `/releases` list (first 30 is plenty). Wrapping the
    // singleton in a Vec keeps the downstream filter chain uniform.
    let url = if include_prerelease {
        "https://api.github.com/repos/packetThrower/zorite/releases?per_page=30"
    } else {
        "https://api.github.com/repos/packetThrower/zorite/releases/latest"
    };

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(5))
        .user_agent(concat!(
            "Zorite/",
            env!("CARGO_PKG_VERSION"),
            " (https://github.com/packetThrower/zorite)"
        ))
        .build();
    let response = match agent.get(url).call() {
        Ok(r) => r,
        Err(err) => {
            log::info!("update check: HTTP {err}");
            return None;
        }
    };
    let parsed = if include_prerelease {
        response.into_json::<Vec<Release>>()
    } else {
        response.into_json::<Release>().map(|r| vec![r])
    };
    match parsed {
        Ok(v) => Some(v),
        Err(err) => {
            log::info!("update check: parse failed: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn current_version_parses_as_semver() {
        // The boot-time check feeds `CARGO_PKG_VERSION` into `check_for_update`;
        // make sure it's parseable semver.
        let raw = env!("CARGO_PKG_VERSION");
        semver::Version::parse(raw).expect("CARGO_PKG_VERSION must be valid semver");
    }
}
