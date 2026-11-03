//! Watches the content sources and rebuilds `data/pack` after changes
//! settle, on its own thread. The World reloads
//! on `DevMsg::Reload`. Once at startup and after every build it also writes
//! `data/symbols.json` (the reference for runescript-lsp's `symbols_match_golden`).

use crate::util::config::Build;
use notify::{RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

/// Watched directories under `build.srcDir`.
const DIRS: [&str; 12] = ["maps", "songs", "jingles", "binary", "fonts", "title", "scripts", "sprites", "models", "textures", "synth", "wordenc"];
const SETTLE: Duration = Duration::from_secs(1);

pub enum DevMsg {
    /// `dev_reload`: the build succeeded.
    Reload,
    /// `dev_failure`.
    Failure(String),
}

pub struct DevWatcher {
    pub messages: Receiver<DevMsg>,
    events: Sender<()>,
    _watcher: notify::RecommendedWatcher,
}

impl DevWatcher {
    /// Starts watching `root/build.srcDir` (creating missing directories).
    pub fn spawn(root: PathBuf, build: Build) -> Result<DevWatcher, String> {
        let (ev_tx, ev_rx) = channel();
        let tx = ev_tx.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok_and(|e| !e.kind.is_access()) {
                let _ = tx.send(());
            }
        })
        .map_err(|e| e.to_string())?;
        let src = root.join(&build.src_dir);
        for dir in DIRS {
            let d = src.join(dir);
            std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
            watcher.watch(&d, RecursiveMode::Recursive).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let (msg_tx, msg_rx) = channel();
        std::thread::Builder::new().name("dev".into()).spawn(move || run(root, build, ev_rx, msg_tx)).map_err(|e| e.to_string())?;
        Ok(DevWatcher { messages: msg_rx, events: ev_tx, _watcher: watcher })
    }

    /// `World.rebuild()` (`world_rebuild`): build after the settle delay.
    pub fn rebuild(&self) {
        let _ = self.events.send(());
    }
}

/// Refreshes `data/symbols.json`. Also after a failed build: a script error
/// still leaves fresh config tables, which is when the editor needs them most.
fn dump_symbols(root: &std::path::Path, build: &Build) {
    match super::all::write_symbols(root, build) {
        Ok(path) => tracing::debug!("wrote {}", path.display()),
        Err(e) => tracing::warn!("symbols.json not written: {e}"),
    }
}

fn run(root: PathBuf, build: Build, events: Receiver<()>, out: Sender<DevMsg>) {
    if root.join("data/pack/server").is_dir() {
        dump_symbols(&root, &build);
    }
    // events that arrive during a build are queued and start the next one
    while events.recv().is_ok() {
        loop {
            match events.recv_timeout(SETTLE) {
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let msg = match super::all::pack_all(&root, &build, ares_compiler::compiler::SourceNames::Absolute) {
            Ok(()) => DevMsg::Reload,
            Err(e) => DevMsg::Failure(e),
        };
        dump_symbols(&root, &build);
        if out.send(msg).is_err() {
            return;
        }
    }
}
