//! Opens a pushed URL in the default browser via `xdg-open`. Phase 2,
//! see `ROADMAP.md`. See `PROTOCOL.md` §5.9 — scheme allow-list is
//! mandatory, and the process MUST be spawned via `Command`
//! arguments, never a shell string, to avoid injection through a
//! crafted URL.

const ALLOWED_SCHEMES: &[&str] = &["http", "https", "file"];

#[derive(Debug, thiserror::Error)]
pub enum UrlPushError {
    #[error("scheme not allowed: {0}")]
    SchemeNotAllowed(String),
    #[error("failed to launch xdg-open: {0}")]
    Spawn(#[from] std::io::Error),
}

pub fn open_url(url: &str) -> Result<(), UrlPushError> {
    let scheme = url.split(':').next().unwrap_or_default().to_lowercase();
    if !ALLOWED_SCHEMES.contains(&scheme.as_str()) {
        return Err(UrlPushError::SchemeNotAllowed(scheme));
    }
    std::process::Command::new("xdg-open").arg(url).spawn()?;
    Ok(())
}
