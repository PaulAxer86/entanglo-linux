//! Short notification sounds from the freedesktop sound theme.
//!
//! Playback runs on a throwaway thread through whichever command-line player
//! is installed, so a missing player or sound file only means silence.

use std::cell::RefCell;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const THEME_DIRECTORY: &str = "/usr/share/sounds/freedesktop/stereo";
const PLAYERS: [&str; 2] = ["pw-play", "paplay"];
/// A failing transfer reports one error per chunk; never repeat a cue faster
/// than this.
const MIN_REPEAT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cue {
    Connected,
    Disconnected,
    Error,
    FileReceived,
    FileSent,
}

impl Cue {
    fn file_name(self) -> &'static str {
        match self {
            Cue::Connected => "bell.oga",
            Cue::Disconnected => "network-connectivity-lost.oga",
            Cue::Error => "dialog-error.oga",
            Cue::FileReceived => "complete.oga",
            Cue::FileSent => "message.oga",
        }
    }
}

#[derive(Default)]
pub struct SoundPlayer {
    last_played: RefCell<HashMap<Cue, Instant>>,
}

impl SoundPlayer {
    pub fn play(&self, cue: Cue) {
        if self.should_play(cue, Instant::now()) {
            let path = format!("{THEME_DIRECTORY}/{}", cue.file_name());
            std::thread::spawn(move || play_file(&path));
        }
    }

    fn should_play(&self, cue: Cue, now: Instant) -> bool {
        let mut last_played = self.last_played.borrow_mut();
        if last_played.get(&cue).is_some_and(|last| now.duration_since(*last) < MIN_REPEAT) {
            return false;
        }
        last_played.insert(cue, now);
        true
    }
}

fn play_file(path: &str) {
    for player in PLAYERS {
        let status = Command::new(player)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if status.is_ok_and(|status| status.success()) {
            return;
        }
    }
    tracing::debug!(path, "no sound player could play the notification");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_cue_is_throttled_until_the_interval_passes() {
        let player = SoundPlayer::default();
        let start = Instant::now();
        assert!(player.should_play(Cue::Error, start));
        assert!(!player.should_play(Cue::Error, start + Duration::from_millis(100)));
        assert!(player.should_play(Cue::Error, start + MIN_REPEAT));
    }

    #[test]
    fn different_cues_do_not_throttle_each_other() {
        let player = SoundPlayer::default();
        let start = Instant::now();
        assert!(player.should_play(Cue::Connected, start));
        assert!(player.should_play(Cue::FileReceived, start));
    }
}
