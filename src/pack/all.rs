//! One clean build of `data/pack`.
//!
//! Step order (it fixes the cache sector layout): revalidate the packs,
//! configs (cache 0/2), interface (0/3), server scripts, title (0/1), media
//! (0/4), textures (0/6), wordenc (0/7), sounds (0/8), models and anims
//! (archives 1, 2), midis (3), maps (4) + worldmap, model flags, versionlist
//! (0/5). Paths are relative to `root`, the engine directory.

use crate::io::filestream::FileStream;
use crate::pack::pack_file::{REGENERATED, revalidate};
use crate::pack::{compiler, config, graphics, interface, map, sprite, versionlist, worldmap};
use crate::util::config::Build;
use ares_compiler::compiler::SourceNames;
use std::path::{Path, PathBuf};

/// Where dev mode writes the compiler symbol tables, under the server root.
pub const SYMBOLS: &str = "data/symbols.json";

/// Remove `data/pack`, the regenerable packs and `data/symbols.json`.
pub fn clean(root: &Path, b: &Build) -> std::io::Result<()> {
    let rm = |p: &Path| -> std::io::Result<()> {
        if p.is_dir() {
            std::fs::remove_dir_all(p)
        } else if p.exists() {
            std::fs::remove_file(p)
        } else {
            Ok(())
        }
    };
    rm(&root.join("data/pack"))?;
    for kind in REGENERATED {
        rm(&root.join(&b.src_dir).join(format!("pack/{kind}.pack")))?;
    }
    rm(&root.join(SYMBOLS))
}

/// `b` with `srcDir` resolved against `root` (srcDir and data/ resolve against root).
fn rooted(root: &Path, b: &Build) -> Build {
    let mut b = b.clone();
    let src = Path::new(&b.src_dir);
    if src.is_relative() {
        b.src_dir = root.join(src).to_string_lossy().replace('\\', "/");
    }
    b
}

/// Writes [`SYMBOLS`]: the symbol tables the script compiler gets, built from
/// the configs and interfaces in `data/pack` (so it needs a build that got
/// that far). For editor tooling (`runescript.symbolsPath`). Written to a temp
/// file and renamed so a reader never sees half a file.
pub fn write_symbols(root: &Path, b: &Build) -> Result<PathBuf, String> {
    let out = root.join("data/pack");
    if !out.join("server").is_dir() {
        return Err("data/pack/server is missing: build first".into());
    }
    let json = serde_json::to_string_pretty(&compiler::symbols(&rooted(root, b), &out)?).map_err(|e| e.to_string())?;
    let path = root.join(SYMBOLS);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json + "\n").map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// A clean build. `source_names` is how scripts name their source file
/// (`Absolute`; tests pin a root).
pub fn pack_all(root: &Path, b: &Build, source_names: SourceNames) -> Result<(), String> {
    let b = rooted(root, b);
    let out = root.join("data/pack");
    let packs = revalidate(&b)?;
    let mut cache = FileStream::open(&out, false).map_err(|e| format!("data/pack: {e}"))?;

    let mut flags = config::pack_configs(&b, &packs, &out, &mut cache, vec![0; packs.model.max.max(0) as usize])?;
    interface::pack_client_interface(&b, &packs, &out, &mut cache, &mut flags)?;

    let output = compiler::run_server_compiler(&b, &out, source_names)?;
    let rendered = output.render_diagnostics();
    if output.stopped.is_some() {
        tracing::error!("
{rendered}");
    } else if !rendered.is_empty() {
        tracing::warn!("
{rendered}");
    }
    if let Some(stage) = output.stopped {
        return Err(format!("script compilation stopped at {stage:?}"));
    }

    sprite::pack_client_title(&b, &out, &mut cache)?;
    sprite::pack_client_media(&b, &out, &mut cache)?;
    sprite::pack_client_texture(&b, &packs, &out, &mut cache)?;
    sprite::pack_client_wordenc(&b, &root.join("data/raw/wordenc"), &mut cache)?;
    sprite::pack_client_sound(&b, &packs.synth, &out, &mut cache)?;
    graphics::pack_client_graphics(&b, &packs, &mut cache, &flags)?;
    graphics::pack_client_midi(&b, &packs, &mut cache)?;
    let server_maps = map::pack_maps(&b, &packs, &out, &mut cache, &mut flags)?;
    if Path::new(&format!("{}/maps", b.src_dir)).exists() {
        worldmap::pack_worldmap(&b, &packs, &out, &server_maps)?;
    }
    let flags = versionlist::model_flags(&flags, packs.model.max);
    versionlist::pack_client_versionlist(&b, &packs, &out, &mut cache, &flags)
}
