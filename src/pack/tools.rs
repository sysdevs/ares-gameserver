//! The one-off utilities outside the build path: checksum, key generation
//! and the NPC/OBJ csv imports.

use crate::io::filestream::FileStream;
use crate::io::jag_names::KNOWN_NAMES;
use crate::io::jagfile::{Jagfile, gen_hash};
use crate::io::packet::Packet;
use crate::pack::name_map::read_ascii;
use crate::pack::parse::{basename_ext, int, int_at, list_dir, load_file};
use crate::util::config::Build;
use std::path::Path;

/// CRC of each archive-0 jag in `data/unpack` and of each named
/// entry, saving the entries to `data/unpack/<jag>/<name>`. Returns the lines printed.
pub fn checksum(root: &Path) -> Result<Vec<String>, String> {
    let dir = root.join("data/unpack");
    let mut cache = FileStream::open(&dir, false).map_err(|e| format!("data/unpack: {e}"))?;
    let mut lines = Vec::new();
    for (file, jag_name) in [(1, "title"), (2, "config"), (3, "interface"), (4, "media"), (5, "versionlist"), (6, "textures"), (7, "wordenc"), (8, "synth")] {
        let src = cache.read(0, file, false).ok_or_else(|| format!("cache 0/{file} is missing"))?;
        lines.push(format!("{jag_name} {}", Packet::getcrc(&src, 0, src.len())));
        let jag = Jagfile::from_bytes(src);
        for i in 0..jag.len() {
            // `jag.fileName` holds only the KNOWN_NAMES matches; holes would be `read(undefined)`
            let Some(name) = KNOWN_NAMES.iter().find(|n| gen_hash(n) == jag.hash(i)) else {
                return Err(format!("{jag_name} entry {i} has no known name"));
            };
            let data = jag.read(name).unwrap().data;
            lines.push(format!("{jag_name} {name} {}", Packet::getcrc(&data, 0, data.len())));
            let path = dir.join(jag_name).join(name);
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&path, &data).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(lines)
}

/// A new 1024-bit key as `data/config/{public,private}.pem`
/// (SPKI public key, PKCS#1 private key). Returns the exponent and
/// modulus lines printed.
pub fn keygen(root: &Path) -> Result<Vec<String>, String> {
    let k = super::keys::generate()?;
    super::keys::write_pems(root, &k)?;
    Ok(vec![
        format!("static readonly exponent: bigint = {}n;", k.e),
        format!("static readonly modulus: bigint = {}n;", k.n),
        "the web client still has the old key: use ares-pack rekey <Client-TS dir> to rebuild it with this one".into(),
    ])
}

struct Spawn {
    id: i64,
    level: i64,
    quantity: i64,
    mapsquare_x: i64,
    mapsquare_z: i64,
    local_x: i64,
    local_z: i64,
    source: String,
}

/// `x,z,level,id[,quantity]` rows; the first line is the header and `//` lines are comments.
fn read_spawns(csv_file: &str, with_quantity: bool) -> Result<Vec<Spawn>, String> {
    let text = read_ascii(csv_file).map_err(|e| format!("{csv_file}: {e}"))?;
    let lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).enumerate().skip(1).filter(|(_, l)| !l.is_empty());
    let mut spawns = Vec::new();
    for (index, (n, line)) in lines.enumerate() {
        if line.starts_with("//") {
            continue;
        }
        let at = format!("{csv_file}:{}", n + 1);
        let csv: Vec<i64> = line.split(',').map(|v| int_at(v, &at)).collect::<Result<_, _>>()?;
        let (x, z, level, id, quantity) = match csv[..] {
            [x, z, level, id] => (x, z, level, id, None),
            [x, z, level, id, q] => (x, z, level, id, Some(q)),
            _ => return Err(format!("{at}: expected x,z,level,id[,quantity]: {line}")),
        };
        let quantity = match (with_quantity, quantity) {
            (true, None) => return Err(format!("{at}: missing quantity: {line}")),
            (_, q) => q.unwrap_or(0),
        };
        spawns.push(Spawn { id, level, quantity, mapsquare_x: x >> 6, mapsquare_z: z >> 6, local_x: x & 63, local_z: z & 63, source: format!("{line}|{index}") });
    }
    Ok(spawns)
}

/// Rewrite each map's NPC (or OBJ)
/// section from a csv of `x,z,level,id[,quantity]` world coordinates.
/// Returns the spawns that matched no map (`source` strings).
pub fn import_csv(b: &Build, csv_file: &str, objs: bool) -> Result<Vec<String>, String> {
    let mut all = read_spawns(csv_file, objs)?;
    let maps_dir = format!("{}/maps", b.src_dir);
    for path in list_dir(&maps_dir) {
        if path.ends_with('/') || !path.ends_with(".jm2") {
            continue;
        }
        let file = &path[path.rfind('/').unwrap() + 1..];
        let mut lines = load_file(&path);
        // `m<mx>_<mz>.jm2`
        let xz = basename_ext(file, ".jm2").strip_prefix('m').and_then(|s| s.split_once('_'));
        let Some((mx, mz)) = xz.and_then(|(x, z)| Some((int(x)?, int(z)?))) else {
            return Err(format!("bad mapsquare name {path}"));
        };

        let (mut here, rest): (Vec<Spawn>, Vec<Spawn>) = all.into_iter().partition(|s| s.mapsquare_x == mx && s.mapsquare_z == mz);
        all = rest;
        here.sort_by_key(|s| (s.level, s.local_x, s.local_z));

        let body: Vec<String> = here
            .iter()
            .map(|s| {
                let base = format!("{} {} {}: {}", s.level, s.local_x, s.local_z, s.id);
                if objs { format!("{base} {}", s.quantity) } else { base }
            })
            .collect();
        let pos = |lines: &[String], l: &str, from: usize| lines.iter().skip(from).position(|x| x == l).map(|i| i + from);

        if objs {
            if let Some(start) = pos(&lines, "==== OBJ ====", 0) {
                lines.truncate(start);
            }
            if !here.is_empty() {
                lines.push("==== OBJ ====".into());
                lines.extend(body);
                lines.push(String::new());
            }
        } else {
            if let Some(start) = pos(&lines, "==== NPC ====", 0) {
                match pos(&lines, "==== OBJ ====", start) {
                    Some(end) => {
                        lines.drain(start..end);
                    }
                    None => lines.truncate(start),
                }
            }
            if !here.is_empty() {
                let tail = match pos(&lines, "==== OBJ ====", 0) {
                    Some(o) => lines.split_off(o),
                    None => Vec::new(),
                };
                lines.push("==== NPC ====".into());
                lines.extend(body);
                lines.push(String::new());
                lines.extend(tail);
            }
        }
        // written back under the maps root by file name
        let out = format!("{maps_dir}/{file}");
        std::fs::write(&out, lines.join("\n")).map_err(|e| format!("{out}: {e}"))?;
    }
    Ok(all.into_iter().map(|s| s.source).collect())
}
