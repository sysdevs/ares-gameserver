//! A clean build of the models, anims and midis: every `.ob2` (archive 1), `.anim` (archive 2) and `.mid` (archive 3,
//! jingles then songs) gzipped in walk order and written with version 1.
//! Empty source files stay empty; names without a pack id are skipped.

use crate::io::filestream::FileStream;
use crate::io::gzip::compress_gz;
use crate::pack::pack_file::{PackFile, Packs};
use crate::pack::parse::{basename_ext, list_files_ext};
use crate::util::config::Build;

fn pack_files(cache: &mut FileStream, archive: usize, files: &[String], ext: &str, pack: &PackFile) -> Result<(), String> {
    // read and gzip in parallel; write serially in walk order (the cache layout)
    let packed = crate::pack::par_map(files, |file| {
        let data = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
        Ok::<_, String>(if data.is_empty() { Vec::new() } else { compress_gz(&data) })
    });
    for (file, packed) in files.iter().zip(packed) {
        let packed = packed?;
        if let Some(id) = pack.get_by_name(basename_ext(file, ext)) {
            cache.write(archive, id as usize, &packed, 1);
        }
    }
    Ok(())
}

/// `packClientGraphics`: models into archive 1, anims into archive 2.
pub fn pack_client_graphics(b: &Build, packs: &Packs, cache: &mut FileStream, model_flags: &[i32]) -> Result<(), String> {
    let models_dir = format!("{}/models", b.src_dir);
    pack_files(cache, 1, &list_files_ext(&models_dir, ".ob2"), ".ob2", &packs.model)?;
    if model_flags.iter().any(|&f| f > 0) {
        for id in 0..packs.model.max {
            if !cache.has(1, id as usize) && model_flags.get(id as usize).is_some_and(|&f| f > 0) {
                tracing::warn!("missing model {} ({id})", packs.model.get_by_id(id));
            }
        }
    }
    pack_files(cache, 2, &list_files_ext(&models_dir, ".anim"), ".anim", &packs.anim_set)
}

/// `packClientMidi`: jingles then songs into archive 3.
pub fn pack_client_midi(b: &Build, packs: &Packs, cache: &mut FileStream) -> Result<(), String> {
    let mut midis = list_files_ext(&format!("{}/jingles", b.src_dir), ".mid");
    midis.extend(list_files_ext(&format!("{}/songs", b.src_dir), ".mid"));
    pack_files(cache, 3, &midis, ".mid", &packs.midi)
}
