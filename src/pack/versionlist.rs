//! The versionlist jag and the model flags.
//!
//! Model flags are the OR of the idk/loc/npc/obj/spotanim config
//! contributions, the interface pack and the map npcs, accumulated while
//! packing; [`model_flags`] only truncates them to the model pack size.
//!
//! The `*_crc` entries are CRCs of the stored gzip bytes, which depend on
//! the zlib implementation (B.3); everything else is byte-exact.

use crate::io::filestream::FileStream;
use crate::io::jagfile::Jagfile;
use crate::io::packet::Packet;
use crate::pack::pack_file::Packs;
use crate::pack::parse::{basename_ext, list_files_ext};
use crate::util::config::Build;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The `modelFlags` array `packClientVersionList` sees after `rebuildModelFlags`.
pub fn model_flags(accumulated: &[i32], model_max: i64) -> Vec<i32> {
    (0..model_max.max(0) as usize).map(|i| accumulated.get(i).copied().unwrap_or(0) & 0xff).collect()
}

/// `packClientVersionList(cache, modelFlags)`.
pub fn pack_client_versionlist(b: &Build, packs: &Packs, out: &Path, cache: &mut FileStream, model_flags: &[i32]) -> Result<(), String> {
    let mut jag = Jagfile::new(true);

    // version, crc and (for models) the flag index of each archive member
    let mut members = |archive: usize, max: i64, mut each: Box<dyn FnMut(i64, Option<&[u8]>) -> Result<(), String> + '_>| -> Result<(Packet, Packet), String> {
        let (mut version, mut crc) = (Packet::alloc(3), Packet::alloc(4));
        for id in 0..max {
            let data = cache.read(archive, id as usize, false);
            match &data {
                Some(d) => {
                    version.p2(1);
                    crc.p4(Packet::getcrc(d, 0, d.len().saturating_sub(2)) as i64);
                }
                None => {
                    version.p2(0);
                    crc.p4(0);
                }
            }
            each(id, data.as_deref())?;
        }
        Ok((version, crc))
    };

    let mut model_index = Packet::alloc(3);
    let (v, c) = members(1, packs.model.max, Box::new(|id, data| {
        model_index.p1(if data.is_some() { model_flags.get(id as usize).copied().unwrap_or(0) as i64 } else { 0 });
        Ok(())
    }))?;
    jag.write("model_version", v.written());
    jag.write("model_crc", c.written());
    jag.write("model_index", model_index.written());

    // frameBase: Uint16Array(AnimPack.max), writes past the end dropped
    let mut frame_base = vec![0u16; packs.anim.max.max(0) as usize];
    let anim_files: HashMap<String, String> =
        list_files_ext(&format!("{}/models", b.src_dir), ".anim").into_iter().map(|f| (basename_ext(&f, ".anim").to_string(), f)).collect();
    let (v, c) = members(2, packs.anim_set.max, Box::new(|id, data| {
        if data.is_some() {
            let name = packs.anim_set.get_by_id(id);
            let file = anim_files.get(name).ok_or_else(|| format!("no .anim file for {name}"))?;
            let mut anim = Packet::new(std::fs::read(file).map_err(|e| format!("{file}: {e}"))?);
            let frames = anim.g2();
            for _ in 0..frames {
                let frame = anim.g2() as usize;
                if let Some(slot) = frame_base.get_mut(frame) {
                    *slot = (id + 1) as u16;
                }
                anim.pos += 1; // group count
            }
        }
        Ok(())
    }))?;
    let mut anim_index = Packet::alloc(3);
    for &f in &frame_base {
        anim_index.p2(f as i64);
    }
    jag.write("anim_version", v.written());
    jag.write("anim_crc", c.written());
    jag.write("anim_index", anim_index.written());

    let mut midi_index = Packet::alloc(3);
    let (v, c) = members(3, packs.midi.max, Box::new(|id, data| {
        if data.is_some() {
            let jingle = format!("{}/jingles/{}.mid", b.src_dir, packs.midi.get_by_id(id));
            midi_index.pbool(Path::new(&jingle).exists());
        } else {
            midi_index.p1(0);
        }
        Ok(())
    }))?;
    jag.write("midi_version", v.written());
    jag.write("midi_crc", c.written());
    jag.write("midi_index", midi_index.written());

    let (v, c) = members(4, packs.map.max, Box::new(|_, _| Ok(())))?;

    // prefetch: the `mx`, `mz` of each zone line's first coord (`level_mx_mz_...`), as `(mx << 8) | mz`
    let path = format!("{}/maps/free2play.csv", b.src_dir);
    let free2play = crate::pack::name_map::read_ascii(&path).map_err(|e| format!("free2play.csv: {e}"))?;
    let mut prefetch = HashSet::new();
    for (i, line) in free2play.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).enumerate() {
        if line.starts_with("//") || line.is_empty() {
            continue;
        }
        let mut parts = line.split('_').skip(1);
        let mut part = || crate::pack::parse::int_at(parts.next().unwrap_or(""), format_args!("{path}:{}", i + 1));
        prefetch.insert((part()? << 8) | part()?);
    }
    let mut map_index = Packet::alloc(4);
    for map_x in 0..100 {
        for map_z in 0..255 {
            let Some(map_id) = packs.map.get_by_name(&format!("m{map_x}_{map_z}")) else { continue };
            let region = (map_x << 8) | map_z;
            map_index.p2(region);
            map_index.p2(map_id);
            // a missing loc square is id -1
            map_index.p2(packs.map.get_by_name(&format!("l{map_x}_{map_z}")).unwrap_or(-1));
            map_index.pbool(prefetch.contains(&region));
        }
    }
    jag.write("map_version", v.written());
    jag.write("map_crc", c.written());
    jag.write("map_index", map_index.written());

    let bytes = jag.to_bytes();
    std::fs::create_dir_all(out.join("client")).map_err(|e| e.to_string())?;
    std::fs::write(out.join("client/versionlist"), &bytes).map_err(|e| format!("client/versionlist: {e}"))?;
    cache.write(0, 5, &bytes, 0);
    Ok(())
}
