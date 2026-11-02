//! The standalone login, friend and logger servers and their database, each run by its own binary (or spawned by `ares` under
//! `easyStartup`).

pub mod db;
pub mod friend;
pub mod logger;
pub mod login;
pub mod ws;

/// Set by `ares` on the servers it spawns; their stdin is a pipe from it.
pub const PARENT_ENV: &str = "ARES_PARENT";

/// Runs a server future on a fresh tokio runtime, for the bins. A server
/// spawned by `ares` exits when its stdin pipe closes, so it never outlives
/// the world however that ends.
pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    if std::env::var_os(PARENT_ENV).is_some() {
        std::thread::spawn(|| {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            std::process::exit(0);
        });
    }
    tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("tokio runtime").block_on(f)
}
