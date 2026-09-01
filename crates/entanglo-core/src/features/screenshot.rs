//! Full-display PNG capture for the `screenshotRequest` peer Quick
//! Action. Phase 2, see `ROADMAP.md`. Planned backing: `ashpd`
//! (`org.freedesktop.portal.Screenshot`) so it works on both
//! GNOME/Wayland and KDE/Wayland, with an X11 `x11rb` fallback for
//! portal-less sessions. See `PROTOCOL.md` §5.10.

pub const REQUESTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
pub const LONG_SIDE_MAX_PX: u32 = 2048;

pub struct ScreenshotCapturer;

impl ScreenshotCapturer {
    pub fn new() -> Self {
        Self
    }

    /// Captures the main display, scales the long side down to
    /// `LONG_SIDE_MAX_PX`, and returns PNG bytes. Skeleton: not yet
    /// implemented — wire up `ashpd::desktop::screenshot` here.
    pub async fn capture_png(&self) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("screenshot capture not yet implemented — see ROADMAP.md Phase 2")
    }
}

impl Default for ScreenshotCapturer {
    fn default() -> Self {
        Self::new()
    }
}
