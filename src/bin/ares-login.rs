//! `ares-login`: the login server. Run from the server root
//! (`data/config/world.json`, `db.sqlite` or the configured mysql).

use ares_gameserver::server::{login::LoginServer, block_on};
use ares_gameserver::util::config::WorldConfig;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).init();
    let root = std::env::args().nth(1).map_or_else(|| PathBuf::from("."), PathBuf::from);
    let config = WorldConfig::load(&root);
    block_on(async move {
        match LoginServer::new(config, root).await {
            Ok(server) => match server.run().await {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    tracing::error!("{e}");
                    ExitCode::FAILURE
                }
            },
            Err(e) => {
                tracing::error!("{e}");
                ExitCode::FAILURE
            }
        }
    })
}
