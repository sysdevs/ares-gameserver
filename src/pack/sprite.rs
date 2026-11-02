//! Title, media, textures, sounds and chat: the jags built from PNGs and
//! raw files.
//! Each one writes `data/pack/client/<name>`, checks its CRC oracle when
//! `verify` is on (media has none), and stores it in cache archive 0.

use crate::io::filestream::FileStream;
use crate::io::jagfile::Jagfile;
use crate::io::packet::Packet;
use crate::pack::pack_file::{PackFile, Packs};
use crate::pack::parse::{basename_ext, list_files_ext};
use crate::pack::pix::convert_image;
use crate::util::config::Build;
use indexmap::IndexMap;
use std::path::Path;

fn finish(b: &Build, out: &Path, cache: &mut FileStream, name: &str, archive_file: usize, bytes: Vec<u8>, crc: Option<i32>) -> Result<(), String> {
    let path = out.join("client").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, &bytes).map_err(|e| format!("client/{name}: {e}"))?;
    if let Some(crc) = crc
        && b.verify
        && !Packet::checkcrc(&bytes, 0, bytes.len(), crc)
    {
        return Err(format!("{name} checksum mismatch!\nYou can disable this safety check by setting BUILD_VERIFY=false"));
    }
    cache.write(0, archive_file, &bytes, 0);
    Ok(())
}

/// `packClientTitle`: fonts, title art, `title.jpg` verbatim, then the shared index.
pub fn pack_client_title(b: &Build, out: &Path, cache: &mut FileStream) -> Result<(), String> {
    let mut index = Packet::alloc(3);
    let fonts = format!("{}/fonts", b.src_dir);
    let title = format!("{}/title", b.src_dir);
    let mut images = Vec::new();
    for name in ["p11_full", "p12_full", "b12_full", "q8_full"] {
        images.push((name, convert_image(&mut index, &fonts, name)?));
    }
    for name in ["logo", "titlebox", "titlebutton", "runes"] {
        images.push((name, convert_image(&mut index, &title, name)?));
    }
    let jpg = std::fs::read(format!("{}/binary/title.jpg", b.src_dir)).map_err(|e| format!("title.jpg: {e}"))?;

    let mut jag = Jagfile::new(false);
    for (name, data) in &images[..5] {
        jag.write(&format!("{name}.dat"), data.written());
    }
    jag.write("title.dat", &jpg);
    for (name, data) in &images[5..] {
        jag.write(&format!("{name}.dat"), data.written());
    }
    jag.write("index.dat", index.written());
    finish(b, out, cache, "title", 1, jag.to_bytes(), Some(410306098))
}

/// `packClientMedia`: every sprite under `sprites/` (sheets with a `.opt` last).
pub fn pack_client_media(b: &Build, out: &Path, cache: &mut FileStream) -> Result<(), String> {
    let mut index = Packet::alloc(3);
    let dir = format!("{}/sprites", b.src_dir);
    let mut sprites = list_files_ext(&dir, ".png");
    // stable sort: sprites with a meta/<name>.opt file go last
    let has_opt = |s: &String| Path::new(&format!("{dir}/meta/{}.opt", basename_ext(s, ".png"))).exists();
    sprites.sort_by_key(has_opt);
    let mut all: IndexMap<String, Packet> = IndexMap::new();
    for s in &sprites {
        let name = basename_ext(s, ".png").to_string();
        let data = convert_image(&mut index, &dir, &name)?;
        all.insert(name, data);
    }
    let mut jag = Jagfile::new(false);
    jag.write("index.dat", index.written());
    for (name, data) in &all {
        jag.write(&format!("{name}.dat"), data.written());
    }
    finish(b, out, cache, "media", 4, jag.to_bytes(), None)
}

/// `packClientTexture`: textures 0..50 by `texture.pack` id, then the index.
pub fn pack_client_texture(b: &Build, packs: &Packs, out: &Path, cache: &mut FileStream) -> Result<(), String> {
    let mut index = Packet::alloc(3);
    let dir = format!("{}/textures", b.src_dir);
    let mut all = Vec::new();
    for id in 0..50 {
        all.push(convert_image(&mut index, &dir, packs.texture.get_by_id(id))?);
    }
    let mut jag = Jagfile::new(false);
    for (id, data) in all.iter().enumerate() {
        jag.write(&format!("{id}.dat"), data.written());
    }
    jag.write("index.dat", index.written());
    finish(b, out, cache, "textures", 6, jag.to_bytes(), Some(915347346))
}

/// `packClientSound`: `p2 id + synth bytes` in `synth.order` order, then `p2 -1`.
pub fn pack_client_sound(b: &Build, synth: &PackFile, out: &Path, cache: &mut FileStream) -> Result<(), String> {
    let order = crate::pack::name_map::load_order(&format!("{}/pack/synth.order", b.src_dir))?;
    let files = list_files_ext(&format!("{}/synth", b.src_dir), ".synth");
    let mut name_to_file: IndexMap<String, String> = IndexMap::new();
    for f in &files {
        let name = basename_ext(f, ".synth");
        if synth.get_by_name(name).is_some() {
            name_to_file.insert(name.to_string(), f.clone());
        }
    }
    let mut buf = Packet::alloc(5);
    for id in order {
        let name = synth.get_by_id(id);
        if name.is_empty() {
            continue;
        }
        let Some(file) = name_to_file.get(name) else { continue };
        buf.p2(id);
        let data = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
        buf.pdata(&data, 0, data.len());
    }
    buf.p2(-1);
    let mut jag = Jagfile::new(false);
    jag.write("sounds.dat", buf.written());
    finish(b, out, cache, "sounds", 8, jag.to_bytes(), Some(-759577225))
}

/// `packClientWordenc`: `data/raw/wordenc` verbatim into cache (0, 7).
pub fn pack_client_wordenc(b: &Build, raw_wordenc: &Path, cache: &mut FileStream) -> Result<(), String> {
    let packed = std::fs::read(raw_wordenc).map_err(|e| format!("{}: {e}", raw_wordenc.display()))?;
    if b.verify && !Packet::checkcrc(&packed, 0, packed.len(), 1386621111) {
        return Err("wordenc checksum mismatch!\nYou can disable this safety check by setting BUILD_VERIFY=false".into());
    }
    cache.write(0, 7, &packed, 0);
    Ok(())
}
