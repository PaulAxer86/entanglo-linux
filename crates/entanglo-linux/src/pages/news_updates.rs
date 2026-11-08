//! Periodic release checks and verified installation for Linux.
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::path::PathBuf;
use gtk::prelude::*;
use gtk::Widget;
use entanglo_core::update::{UpdateManifest, UpdateService};
use crate::state::AppShared;
use super::layout;

fn install(manifest: UpdateManifest) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is unavailable")?);
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".cache")).join("entanglo/updates");
    let package = UpdateService::download(&manifest, &cache).map_err(|e| e.to_string())?;
    for (field, expected) in [("Package", "entanglo-linux"), ("Architecture", "amd64"), ("Version", manifest.version.as_str())] {
        let output = std::process::Command::new("dpkg-deb").arg("--field").arg(&package).arg(field).output().map_err(|e| e.to_string())?;
        if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != expected { return Err(format!("Unexpected package {field}")); }
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    if executable.starts_with(home.join(".local")) && executable.metadata().map_err(|e| e.to_string())?.uid() == unsafe { libc::geteuid() } {
        let extract = cache.join(format!("extract-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&extract).map_err(|e| e.to_string())?;
        let result = (|| -> Result<(), String> {
            let status = std::process::Command::new("dpkg-deb").arg("--extract").arg(&package).arg(&extract).status().map_err(|e| e.to_string())?;
            if !status.success() { return Err("Could not extract update package".into()); }
            let source = extract.join("usr/bin/entanglo-linux");
            let temporary = executable.with_extension(format!("update-{}",uuid::Uuid::new_v4()));
            std::fs::copy(&source,&temporary).map_err(|e| e.to_string())?;
            std::fs::set_permissions(&temporary,std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
            std::fs::copy(&executable,executable.with_extension("previous")).map_err(|e| e.to_string())?;
            std::fs::rename(&temporary,&executable).map_err(|e| e.to_string())?;
            let icons=extract.join("usr/share/icons/hicolor");
            if let Ok(sizes)=std::fs::read_dir(icons) {
                for size in sizes.flatten() {
                    let source=size.path().join("apps/com.paoloasara.Entanglo.png");
                    if source.is_file() {
                        let target=home.join(".local/share/icons/hicolor").join(size.file_name()).join("apps");
                        if std::fs::create_dir_all(&target).is_ok() { let _=std::fs::copy(source,target.join("com.paoloasara.Entanglo.png")); }
                    }
                }
            }
            Ok(())
        })();
        let _=std::fs::remove_dir_all(extract);
        result?;
    } else {
        let status=std::process::Command::new("pkexec").args(["dpkg","--install"]).arg(&package).status().map_err(|e| e.to_string())?;
        if !status.success() { return Err("Installation was cancelled or failed".into()); }
    }
    // Wait for this instance to quit before registering the replacement.
    std::process::Command::new("sh").args(["-c", "while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done; exec \"$2\"", "entanglo-restart", &std::process::id().to_string()])
        .arg(&executable).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn build(shared: &Rc<AppShared>) -> Widget {
    let column=gtk::Box::new(gtk::Orientation::Vertical,14);
    let version=gtk::Label::new(Some(&format!("Installed version: {}",env!("CARGO_PKG_VERSION"))));
    version.set_xalign(0.0);version.add_css_class("heading");column.append(&version);
    let status=gtk::Label::new(Some("Checking for updates…"));status.set_xalign(0.0);status.set_wrap(true);column.append(&status);
    let details=gtk::Label::new(None);details.set_xalign(0.0);details.set_wrap(true);details.set_selectable(true);column.append(&details);
    let actions=gtk::Box::new(gtk::Orientation::Horizontal,10);
    let check=gtk::Button::with_label("Check now");actions.append(&check);
    let update=gtk::Button::with_label("Install update");update.add_css_class("suggested-action");update.set_visible(false);actions.append(&update);column.append(&actions);
    let note=gtk::Label::new(Some("Updates are checked on launch and every hour. Downloads are verified by size and SHA-256. System installations request administrator approval; personal installations update in your account."));
    note.set_xalign(0.0);note.set_wrap(true);note.add_css_class("dim-label");column.append(&note);
    column.append(&gtk::LinkButton::with_label("https://entanglo.pages.dev/download", "Downloads and release information"));
    let candidate=Rc::new(RefCell::new(None::<UpdateManifest>));
    let busy=Rc::new(Cell::new(false));
    let launch_check: Rc<dyn Fn()>={
        let status=status.clone();let details=details.clone();let update=update.clone();let candidate=candidate.clone();let busy=busy.clone();let handle=shared.backend.handle.clone();
        Rc::new(move || {
            if busy.replace(true) { return; }
            status.set_text("Checking for updates…");
            let work=handle.spawn(async { UpdateService.check_for_update(env!("CARGO_PKG_VERSION")).await });
            let status=status.clone();let details=details.clone();let update=update.clone();let candidate=candidate.clone();let busy=busy.clone();
            glib::spawn_future_local(async move {
                match work.await {
                    Ok(Ok(Some(manifest))) => {
                        status.set_text(&format!("Entanglo {} is available",manifest.version));
                        details.set_text(&format!("{}\n{}",manifest.title,manifest.highlights.join("\n")));
                        update.set_visible(true);*candidate.borrow_mut()=Some(manifest);
                    }
                    Ok(Ok(None)) => { status.set_text("You are up to date");details.set_text("");update.set_visible(false);*candidate.borrow_mut()=None; }
                    result => { status.set_text("Could not check for updates. You can retry or use the download page.");tracing::info!(?result,"update check unavailable"); }
                }
                busy.set(false);
            });
        })
    };
    let trigger=launch_check.clone();check.connect_clicked(move |_| trigger());
    let state=shared.clone();let status_update=status.clone();let busy_update=busy.clone();
    update.connect_clicked(move |button| {
        if busy_update.replace(true) { return; }
        let Some(manifest)=candidate.borrow().clone() else { busy_update.set(false);return; };
        status_update.set_text("Downloading and verifying update…");button.set_sensitive(false);
        let work=state.backend.handle.spawn_blocking(move || install(manifest));
        let status=status_update.clone();let button=button.clone();let busy=busy_update.clone();
        glib::spawn_future_local(async move {
            match work.await {
                Ok(Ok(())) => { status.set_text("Update installed. Restarting…");if let Some(app)=gtk::gio::Application::default(){app.quit();} }
                result => { status.set_text(&format!("Update failed: {result:?}"));button.set_sensitive(true);busy.set(false); }
            }
        });
    });
    launch_check();
    let weak=column.downgrade();
    glib::timeout_add_local(std::time::Duration::from_secs(3600),move || {
        if weak.upgrade().is_none(){return glib::ControlFlow::Break;}
        launch_check();glib::ControlFlow::Continue
    });
    layout::page("News & Updates","Linux releases, verified downloads and automatic update checks.",&column)
}
