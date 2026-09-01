//! Permission status + local identity. Mirrors
//! `entanglo-macos/docs/PERMISSIONS.md`'s posture: never bypass a
//! missing permission, always say exactly which switch to flip.
//! `ROADMAP.md` Phase 1's "first-run onboarding" item, done as a
//! Settings page section rather than a first-run dialog.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Label, Orientation, Widget};

use crate::state::AppShared;

pub fn build(shared: &Rc<AppShared>) -> Widget {
    let container = gtk::Box::new(Orientation::Vertical, 8);
    container.set_margin_top(16);
    container.set_margin_bottom(16);
    container.set_margin_start(16);
    container.set_margin_end(16);

    let username = current_username();
    let in_group_config = username
        .as_deref()
        .map(is_in_input_group_config)
        .unwrap_or(false);
    let (controller_enabled, device_count) = shared.backend.coordinator.controller_status();
    let receiver_enabled = shared.backend.coordinator.receiver_enabled();
    let process_has_access = controller_enabled || receiver_enabled;

    let config_label = Label::new(Some(&format!(
        "'input' group (system config): {}",
        if in_group_config { "yes" } else { "no" }
    )));
    config_label.set_halign(gtk::Align::Start);
    container.append(&config_label);

    let process_label = Label::new(Some(&format!(
        "'input' group (this running app): {}",
        if process_has_access { "yes" } else { "no" }
    )));
    process_label.set_halign(gtk::Align::Start);
    container.append(&process_label);

    // Three distinct states, each needing a different instruction —
    // never silently degrade, always say exactly what to do next.
    let guidance = if in_group_config && process_has_access {
        None
    } else if in_group_config && !process_has_access {
        Some(
            "Group membership was granted after this app's current \
             session started. Log out and back in (group membership \
             is read at login), then restart Entanglo."
                .to_string(),
        )
    } else {
        Some(format!(
            "Input sharing needs the 'input' group. Run:\n    sudo usermod -aG input {}\nthen log out and back in.",
            username.as_deref().unwrap_or("$USER")
        ))
    };
    if let Some(text) = guidance {
        let label = Label::new(Some(&text));
        label.set_halign(gtk::Align::Start);
        label.set_wrap(true);
        label.add_css_class("dim-label");
        container.append(&label);
    }

    let devices_label = Label::new(Some(&format!(
        "Local input devices detected: {device_count}"
    )));
    devices_label.set_halign(gtk::Align::Start);
    container.append(&devices_label);

    container.append(&gtk::Separator::new(Orientation::Horizontal));

    let identity_label = Label::new(Some(&format!(
        "Device id: {}",
        shared.backend.local_device_id
    )));
    identity_label.set_halign(gtk::Align::Start);
    identity_label.set_selectable(true);
    container.append(&identity_label);

    container.into()
}

fn current_username() -> Option<String> {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .ok()
}

/// Whether `/etc/group`'s `input:` line lists `username` as a
/// supplementary member. Deliberately reads the system config file
/// rather than this process's own live group list — the two can
/// disagree right after `usermod`, and that disagreement is exactly
/// the "log out and back in" case this page needs to explain.
fn is_in_input_group_config(username: &str) -> bool {
    std::fs::read_to_string("/etc/group")
        .map(|contents| group_contains_member(&contents, "input", username))
        .unwrap_or(false)
}

fn group_contains_member(etc_group_contents: &str, group_name: &str, username: &str) -> bool {
    for line in etc_group_contents.lines() {
        // /etc/group line shape: name:password:GID:member1,member2,...
        let mut fields = line.splitn(4, ':');
        if fields.next() != Some(group_name) {
            continue;
        }
        let members = fields.nth(2).unwrap_or("");
        return members.split(',').any(|m| m == username);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_member_in_target_group() {
        let etc_group = "sudo:x:27:alice\ninput:x:996:pasura,bob\naudio:x:29:pasura\n";
        assert!(group_contains_member(etc_group, "input", "pasura"));
        assert!(group_contains_member(etc_group, "input", "bob"));
    }

    #[test]
    fn does_not_find_member_of_a_different_group() {
        let etc_group = "sudo:x:27:alice\ninput:x:996:bob\naudio:x:29:pasura\n";
        assert!(!group_contains_member(etc_group, "input", "pasura"));
    }

    #[test]
    fn missing_group_line_is_not_a_match() {
        let etc_group = "sudo:x:27:alice\naudio:x:29:pasura\n";
        assert!(!group_contains_member(etc_group, "input", "pasura"));
    }

    #[test]
    fn empty_member_list_is_not_a_match() {
        let etc_group = "input:x:996:\n";
        assert!(!group_contains_member(etc_group, "input", "pasura"));
    }
}
