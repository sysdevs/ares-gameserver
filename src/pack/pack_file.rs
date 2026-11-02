//! The `<srcDir>/pack/<type>.pack` id ↔ name maps and their validators.
//!
//! Every build is clean, so every pack is revalidated.
//! Id assignment for new names follows the bytewise walk order (B.1).

use crate::pack::parse::{basename, basename_ext, dirname, inner, list_files_ext, load_dir_ext_full, load_file};
use crate::util::config::Build;
use indexmap::{IndexMap, IndexSet};
use std::collections::HashMap;

/// The packs a clean deletes: they are regenerated from the sources.
pub const REGENERATED: &[&str] = &["category", "enum", "param", "script", "struct", "mesanim", "dbrow", "dbtable", "hunt", "varn", "vars"];

#[derive(Debug, Default)]
pub struct PackFile {
    pub kind: &'static str,
    /// Insertion-ordered; `save` sorts by id.
    pack: IndexMap<i64, String>,
    names: IndexSet<String>,
    name_to_id: HashMap<String, i64>,
    pub max: i64,
}

impl PackFile {
    pub fn new(kind: &'static str) -> Self {
        PackFile { kind, ..Default::default() }
    }

    pub fn size(&self) -> usize {
        self.pack.len()
    }

    pub fn path(src_dir: &str, kind: &str) -> String {
        format!("{src_dir}/pack/{kind}.pack")
    }

    /// `load(path)`: resets `pack` only (not `names`/`nameToId`/`max`).
    pub fn load(&mut self, path: &str) -> Result<(), String> {
        self.pack = IndexMap::new();
        if !std::path::Path::new(path).exists() {
            return Ok(());
        }
        for (i, line) in load_file(path).iter().enumerate() {
            let digits = line.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 || line.as_bytes().get(digits) != Some(&b'=') {
                continue;
            }
            let mut parts = line.split('=');
            let id = parts.next().unwrap().parse().map_err(|_| format!("{path}:{}: bad id", i + 1))?;
            let name = parts.next().unwrap_or("");
            if name.is_empty() {
                return Err(format!("Pack file has an empty name {path}:{}", i + 1));
            }
            self.register(id, name);
        }
        self.refresh_names();
        Ok(())
    }

    pub fn register(&mut self, id: i64, name: &str) {
        self.pack.insert(id, name.to_string());
        self.name_to_id.insert(name.to_string(), id);
    }

    pub fn refresh_names(&mut self) {
        self.names = self.pack.values().cloned().collect();
        if !self.names.is_empty() {
            self.max = self.pack.keys().copied().max().unwrap() + 1;
        }
    }

    /// The text `save()` writes: `id=name` sorted by id, `\n`-terminated.
    pub fn to_text(&self) -> String {
        let mut entries: Vec<_> = self.pack.iter().collect();
        entries.sort_by_key(|(id, _)| **id);
        entries.iter().map(|(id, name)| format!("{id}={name}")).collect::<Vec<_>>().join("\n") + "\n"
    }

    pub fn save(&self, src_dir: &str) -> Result<(), String> {
        let path = PackFile::path(src_dir, self.kind);
        let text = self.to_text();
        if std::fs::read(&path).is_ok_and(|cur| cur == text.as_bytes()) {
            return Ok(());
        }
        if let Some(dir) = std::path::Path::new(&path).parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, text).map_err(|e| format!("{path}: {e}"))
    }

    /// `names`: the pack's names in insertion order.
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.names.iter()
    }

    pub fn has_name(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// `getById(id)`: `''` when absent.
    pub fn get_by_id(&self, id: i64) -> &str {
        self.pack.get(&id).map_or("", String::as_str)
    }

    pub fn get_by_name(&self, name: &str) -> Option<i64> {
        self.name_to_id.get(name).copied()
    }
}

/// Every pack, in revalidation order.
#[derive(Debug, Default)]
pub struct Packs {
    pub anim_set: PackFile,
    pub anim: PackFile,
    pub base: PackFile,
    pub category: PackFile,
    pub db_row: PackFile,
    pub db_table: PackFile,
    pub enums: PackFile,
    pub flo: PackFile,
    pub hitsplat: PackFile,
    pub hunt: PackFile,
    pub idk: PackFile,
    pub interface: PackFile,
    pub inv: PackFile,
    pub loc: PackFile,
    pub mes_anim: PackFile,
    pub map: PackFile,
    pub midi: PackFile,
    pub model: PackFile,
    pub npc: PackFile,
    pub obj: PackFile,
    pub param: PackFile,
    pub script: PackFile,
    pub seq: PackFile,
    pub synth: PackFile,
    pub spot_anim: PackFile,
    pub structs: PackFile,
    pub texture: PackFile,
    pub varp: PackFile,
    pub varn: PackFile,
    pub vars: PackFile,
    pub varo: PackFile,
    pub varbit: PackFile,
}

/// `revalidatePack()` for a clean build.
pub fn revalidate(b: &Build) -> Result<Packs, String> {
    let src = b.src_dir.as_str();
    let models = format!("{src}/models");
    let files = |kind, paths: &[&str], ext, verify| validate_files(b, PackFile::new(kind), paths, ext, verify);
    let config = |kind, ext, transmitted| validate_config(b, PackFile::new(kind), ext, transmitted);
    Ok(Packs {
        anim_set: files("animset", &[&models], ".anim", true)?,
        anim: files("anim", &[&models], ".frame", false)?,
        base: files("base", &[&models], ".base", false)?,
        category: validate_category(b)?,
        db_row: config("dbrow", ".dbrow", false)?,
        db_table: config("dbtable", ".dbtable", false)?,
        enums: config("enum", ".enum", false)?,
        flo: config("flo", ".flo", true)?,
        hitsplat: config("hitsplat", ".hitsplat", true)?,
        hunt: config("hunt", ".hunt", false)?,
        idk: config("idk", ".idk", true)?,
        interface: validate_interface(b)?,
        inv: config("inv", ".inv", false)?,
        loc: config("loc", ".loc", true)?,
        mes_anim: config("mesanim", ".mesanim", false)?,
        map: files("map", &[&format!("{src}/maps")], ".jm2", false)?,
        midi: files("midi", &[&format!("{src}/jingles"), &format!("{src}/songs")], ".mid", true)?,
        model: files("model", &[&models], ".ob2", true)?,
        npc: config("npc", ".npc", true)?,
        obj: config("obj", ".obj", true)?,
        param: config("param", ".param", false)?,
        script: regen_script(b)?,
        seq: config("seq", ".seq", true)?,
        synth: files("synth", &[&format!("{src}/synth")], ".synth", true)?,
        spot_anim: config("spotanim", ".spotanim", true)?,
        structs: config("struct", ".struct", false)?,
        texture: validate_image(b, PackFile::new("texture"), &format!("{src}/textures"), ".png")?,
        varp: config("varp", ".varp", true)?,
        varn: config("varn", ".varn", false)?,
        vars: config("vars", ".vars", false)?,
        varo: config("varo", ".varo", false)?,
        varbit: config("varbit", ".varbit", true)?,
    })
}

/// File name without directory and extension.
fn stem<'a>(file: &'a str, ext: &str) -> &'a str {
    let name = &file[file.rfind('/').map_or(0, |i| i + 1)..];
    &name[..name.len() - ext.len()]
}

fn validate_files(b: &Build, mut pack: PackFile, paths: &[&str], ext: &str, verify: bool) -> Result<PackFile, String> {
    pack.load(&PackFile::path(&b.src_dir, pack.kind))?;
    for path in paths {
        let files = list_files_ext(path, ext);
        if verify {
            for f in &files {
                let name = stem(f, ext);
                if !pack.has_name(name) {
                    return Err(format!("{}: {name} is missing an ID line, you may need to edit {}/pack/{}.pack", pack.kind, b.src_dir, pack.kind));
                }
            }
            // `verifyPack` only logs a commented-out warning here.
        }
    }
    pack.save(&b.src_dir)?;
    Ok(pack)
}

fn validate_image(b: &Build, mut pack: PackFile, path: &str, ext: &str) -> Result<PackFile, String> {
    pack.load(&PackFile::path(&b.src_dir, pack.kind))?;
    let files = list_files_ext(path, ext);
    let mut file_names: IndexSet<String> = files.iter().map(|f| basename_ext(f, ext).to_string()).collect();
    for f in &files {
        if basename(dirname(f)) == "meta" {
            continue;
        }
        let name = stem(f, ext);
        file_names.insert(name.to_string());
        if !pack.has_name(name) {
            return Err(format!("{}: {name} is missing an ID line, you may need to edit {}/pack/{}.pack", pack.kind, b.src_dir, pack.kind));
        }
    }
    if b.verify_pack
        && let Some(name) = pack.names.iter().find(|n| !file_names.contains(*n))
    {
        return Err(format!("{}: {name} was not found on your disk, you may need to edit {}/pack/{}.pack", pack.kind, b.src_dir, pack.kind));
    }
    pack.save(&b.src_dir)?;
    Ok(pack)
}

/// `crawlConfigNames(ext, includeBrackets)`: every `[name]` header under
/// `<srcDir>/scripts` in walk order, deduplicated (first occurrence wins).
pub fn crawl_config_names(b: &Build, ext: &str, include_brackets: bool) -> Result<Vec<String>, String> {
    let scripts = format!("{}/scripts", b.src_dir);
    let engine = format!("{scripts}/engine.rs2");
    let mut names = IndexSet::new();
    for entry in load_dir_ext_full(&scripts, ext) {
        let (lines, file) = entry?;
        let file = file.as_str();
        if file == engine {
            continue;
        }
        for line in lines.iter().filter(|l| l.starts_with('[')) {
            // up to the first `]`; none gives ''
            let name = line.find(']').map_or("", |i| &line[..=i]);
            let name = if include_brackets { name } else { inner(name) };
            if b.verify_folder {
                let dir = basename(dirname(file));
                let parent = basename(dirname(dirname(file)));
                let parent_parent = basename(dirname(dirname(dirname(file))));
                if dir != "_unpack" && parent != "_unpack" && parent_parent != "_unpack" && ext != ".flo" {
                    if ext == ".rs2" && dir != "scripts" && parent != "scripts" {
                        return Err(format!("Script file {file} must be located inside a \"scripts\" directory."));
                    } else if ext != ".rs2" && dir != "configs" && parent != "configs" {
                        return Err(format!("Config file {file} must be located inside a \"configs\" directory."));
                    }
                }
            }
            names.insert(name.to_string());
        }
    }
    Ok(names.into_iter().collect())
}

fn validate_config(b: &Build, mut pack: PackFile, ext: &str, transmitted: bool) -> Result<PackFile, String> {
    let names = crawl_config_names(b, ext, false)?;
    pack.load(&PackFile::path(&b.src_dir, pack.kind))?;
    let regenerate = !transmitted || !b.verify;
    if regenerate {
        for name in &names {
            if !pack.has_name(name) {
                let id = pack.max;
                pack.max += 1;
                pack.register(id, name);
            }
        }
        pack.refresh_names();
    }

    let missing: Vec<_> = names.iter().filter(|n| !pack.has_name(n) && !n.starts_with("cert_")).collect();
    if b.verify && !missing.is_empty() {
        return Err(format!("Missing {} pack IDs for {missing:?}: you may need to edit {}/pack/{}.pack", pack.kind, b.src_dir, pack.kind));
    }

    if transmitted && b.verify {
        let config_names: IndexSet<&String> = names.iter().collect();
        if let Some(name) = pack.names.iter().find(|n| !config_names.contains(n) && !n.starts_with("cert_")) {
            return Err(format!("{}: {name} was not found in any {ext} files, you may need to edit {}/pack/{}.pack", pack.kind, b.src_dir, pack.kind));
        }
    }

    if regenerate {
        pack.save(&b.src_dir)?;
    }
    Ok(pack)
}

/// `crawlConfigCategories()`: `category=` values from `.loc`, then `.npc`, then `.obj`.
fn validate_category(b: &Build) -> Result<PackFile, String> {
    let scripts = format!("{}/scripts", b.src_dir);
    let mut categories = IndexSet::new();
    for ext in [".loc", ".npc", ".obj"] {
        for entry in load_dir_ext_full(&scripts, ext) {
            for l in &entry?.0 {
                if let Some(c) = l.strip_prefix("category=") {
                    categories.insert(c.to_string());
                }
            }
        }
    }
    // Not loaded first: registers into the (empty) in-memory pack.
    let mut pack = PackFile::new("category");
    for (i, c) in categories.iter().enumerate() {
        pack.register(i as i64, c);
    }
    pack.refresh_names();
    pack.save(&b.src_dir)?;
    Ok(pack)
}

fn validate_interface(b: &Build) -> Result<PackFile, String> {
    let mut pack = PackFile::new("interface");
    pack.load(&PackFile::path(&b.src_dir, "interface"))?;
    let scripts = format!("{}/scripts", b.src_dir);
    for entry in load_dir_ext_full(&scripts, ".if") {
        let (lines, file) = entry?;
        let file = file.as_str();
        if b.verify_folder {
            let parent = basename(dirname(dirname(file)));
            let dir = basename(dirname(file));
            if dir != "interfaces" && parent != "interfaces" {
                return Err(format!("Interface file {file} must be located inside an \"interfaces\" directory."));
            }
        }
        let inter = basename_ext(file, ".if");
        if !pack.has_name(inter) {
            return Err(format!("{}/pack/interface.pack is missing ID for interface {inter} from {file}", b.src_dir));
        }
        for line in lines.iter().filter(|l| l.starts_with('[')) {
            let com = inner(line);
            let name = format!("{inter}:{com}");
            if !pack.has_name(&name) {
                return Err(format!("{}/pack/interface.pack is missing ID for component {name} from {file}", b.src_dir));
            }
        }
    }
    Ok(pack)
}

fn regen_script(b: &Build) -> Result<PackFile, String> {
    let mut pack = PackFile::new("script");
    pack.load(&PackFile::path(&b.src_dir, "script"))?;
    for name in crawl_config_names(b, ".rs2", true)? {
        if !pack.has_name(&name) {
            let id = pack.max;
            pack.max += 1;
            pack.register(id, &name);
        }
    }
    pack.refresh_names();
    pack.save(&b.src_dir)?;
    Ok(pack)
}
