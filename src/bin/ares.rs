//! `ares`: the game world. Run from a server root holding
//! `data/config/world.json`, `data/pack` (built by `ares-pack build`) and
//! `data/config/private.pem`; `build.srcDir` points at the content checkout.
//!
//! `login.enabled=false` keeps saves in-process under `data/players/<profile>`;
//! otherwise the world talks to `ares-login` (and `ares-friend` when
//! `friend.enabled`). `easyStartup` spawns the three server binaries from the
//! directory holding this executable.

use ares_gameserver::engine::runtime::{LoginLink, Node, Runtime};
use ares_gameserver::engine::world::World;
use ares_gameserver::net::tcp;
use ares_gameserver::pack::dev::DevWatcher;
use ares_gameserver::util::config::WorldConfig;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let root = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    let config = WorldConfig::load(&root);
    let mut children = Vec::new();
    if config.easy_startup {
        let dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(PathBuf::from))
            .unwrap_or_default();
        for name in ["ares-login", "ares-friend", "ares-logger"] {
            let mut cmd = std::process::Command::new(dir.join(name));
            cmd.arg(&root)
                .env(ares_gameserver::server::PARENT_ENV, "1")
                .stdin(std::process::Stdio::piped());
            match cmd.spawn() {
                Ok(c) => children.push(c),
                Err(e) => tracing::error!("spawn {name}: {e}"),
            }
        }
    }
    if !root.join("data/pack/server/script.dat").exists() {
        tracing::info!("Packing cache, please wait until you see the world is ready.");
        if let Err(e) = ares_gameserver::pack::all::pack_all(
            &root,
            &config.build,
            ares_compiler::compiler::SourceNames::Absolute,
        ) {
            tracing::error!("{e}");
            return ExitCode::FAILURE;
        }
    }
    tracing::info!("Starting world");
    let world = match World::start(&config, &root) {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("{e}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(
        "{} npcs, {} zones",
        world.npcs.count(),
        world.zones.zone_count()
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let port = config.node.port as u16;
    #[cfg(feature = "web-client")]
    {
        let src = root.join(&config.build.src_dir);
        let web = ares_gameserver::web::WebConfig {
            root: root.clone(),
            src_dir: src,
            port: config.web.port as u16,
            allowed_origin: config.web.allowed_origin.clone(),
            node_id: config.node.id,
            node_port: config.node.port,
            members: config.node.members,
            debug: config.node.debug,
        };
        let web_port = web.port;
        if let Err(e) = ares_gameserver::web::spawn(web, tx.clone()) {
            tracing::error!("web on {web_port}: {e}");
            return ExitCode::FAILURE;
        }
        tracing::info!("Web client on port http://localhost:{web_port}/rs2.cgi");
        if let Err(report) = ares_gameserver::pack::keys::keycheck(&root, false) {
            tracing::warn!("login keys out of sync:
{report}");
        }
    }
    if let Err(e) = tcp::listen(port, tx) {
        tracing::error!("listen on {port}: {e}");
        return ExitCode::FAILURE;
    }
    tracing::info!("World ready: game port {port}");
    let login = if config.login.enabled {
        LoginLink::Ws {
            host: config.login.host.clone(),
            port: config.login.port as u16,
        }
    } else {
        LoginLink::Inline {
            dir: root.join("data/players").join(&config.node.profile),
        }
    };
    let friend = config
        .friend
        .enabled
        .then(|| (config.friend.host.clone(), config.friend.port as u16));
    let node = Node {
        id: config.node.id,
        profile: config.node.profile.clone(),
    };
    let dev = if !config.node.production && config.build.live_reload {
        match DevWatcher::spawn(root.clone(), config.build.clone()) {
            Ok(d) => {
                if config.build.startup {
                    d.rebuild();
                }
                Some((d, format!("{}/scripts/", config.build.src_dir)))
            }
            Err(e) => {
                tracing::error!("live reload: {e}");
                None
            }
        }
    } else {
        None
    };
    let code = match Runtime::new(world, rx, &root.join("data/pack"), node, login, friend, dev) {
        Ok(rt) => {
            rt.run();
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    };
    for mut c in children {
        let _ = c.kill();
    }
    code
}
