//! `ares-pack`: the build tooling (`build` / `clean` and the one-off tools).
//! Runs in the server root: it reads
//! `data/config/world.json` and writes `data/pack`.
//!
//! ```text
//! ares-pack build            clean build of data/pack (Clean + Build)
//! ares-pack clean            remove data/pack and the regenerable packs
//! ares-pack symbols         write data/symbols.json (compiler symbol tables) from a built data/pack; dev mode does this after every build
//! ares-pack checksum         CRCs of the jags in data/unpack
//! ares-pack keygen           new data/config/{public,private}.pem
//! ares-pack rekey <dir>      new key + rebuilt web client from a Client-TS checkout (cloned if missing)
//! ares-pack keycheck [--fix] private.pem, public.pem and public/client/client.js agree (--fix: rewrite public.pem)
//! ares-pack import-npc-csv <file>   rewrite map NPC sections
//! ares-pack import-obj-csv <file>   rewrite map OBJ sections
//! ```

use ares_gameserver::pack::{all, keys, tools};
use ares_gameserver::util::config::WorldConfig;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = Path::new(".");
    let config = WorldConfig::load(root);
    let b = &config.build;
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["build"] => all::clean(root, b).map_err(|e| e.to_string()).and_then(|_| all::pack_all(root, b, ares_compiler::compiler::SourceNames::Absolute)),
        ["clean"] => all::clean(root, b).map_err(|e| e.to_string()),
        ["symbols"] => all::write_symbols(root, b).map(|p| println!("wrote {}", p.display())),
        ["checksum"] => tools::checksum(root).map(|lines| lines.iter().for_each(|l| println!("{l}"))),
        ["keygen"] => tools::keygen(root).map(|lines| lines.iter().for_each(|l| println!("{l}"))),
        ["rekey", dir] => keys::rekey(root, Path::new(dir), config.engine.revision).map(|lines| lines.iter().for_each(|l| println!("{l}"))),
        ["keycheck"] => keys::keycheck(root, false).map(|lines| lines.iter().for_each(|l| println!("{l}"))),
        ["keycheck", "--fix"] => keys::keycheck(root, true).map(|lines| lines.iter().for_each(|l| println!("{l}"))),
        [cmd @ ("import-npc-csv" | "import-obj-csv"), file] => tools::import_csv(b, file, *cmd == "import-obj-csv").map(|left| {
            if !left.is_empty() {
                println!("{} leftover {}:", left.len(), if *cmd == "import-obj-csv" { "objs" } else { "NPCs" });
                left.iter().for_each(|s| println!("  {s}"));
            }
        }),
        _ => {
            eprintln!("usage: ares-pack build | clean | symbols | checksum | keygen | rekey <Client-TS dir> | keycheck [--fix] | import-npc-csv <file> | import-obj-csv <file>");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}
