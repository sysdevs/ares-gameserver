//! `.if` sources → the
//! client `interface` jag (whole-archive bzip2, one `data` entry) and
//! `server/interface.dat` (ids, names, overlay flags).
//!
//! Source lines come from `name_map::load_dir` (`'ascii'`, untrimmed, empty lines
//! dropped) and are stored as raw `key=value` strings with their line; numeric
//! fields are parsed (strictly) at write time, and a missing key writes 0.

use crate::io::filestream::FileStream;
use crate::io::jagfile::Jagfile;
use crate::io::packet::Packet;
use crate::pack::name_map::{load_dir, load_order};
use crate::pack::parse::{inner, int_at};
use crate::pack::pack_file::Packs;
use crate::util::config::Build;
use std::collections::HashMap;
use std::path::Path;

#[derive(Default)]
struct Com {
    root: Option<String>,
    children: Vec<i64>,
    /// `key → (value, file:line)`.
    src: HashMap<String, (String, String)>,
}

impl Com {
    fn get(&self, k: &str) -> Option<&str> {
        self.src.get(k).map(|(v, _)| v.as_str())
    }

    /// `s` (a part of `k`'s value) as an int; `None` (absent) → 0.
    fn num(&self, k: &str, s: Option<&str>) -> Result<i64, String> {
        s.map_or(Ok(0), |s| int_at(s, self.src.get(k).map_or("", |(_, at)| at.as_str())))
    }

    /// JS truthiness of `src[k]` (present and non-empty).
    fn truthy(&self, k: &str) -> bool {
        self.get(k).is_some_and(|v| !v.is_empty())
    }

    fn int(&self, k: &str) -> Result<i64, String> {
        self.num(k, self.get(k))
    }

    fn yes(&self, k: &str) -> bool {
        self.get(k) == Some("yes")
    }
}

fn name_to_type(name: Option<&str>) -> i64 {
    match name {
        Some("layer" | "overlay") => 0,
        Some("inv") => 2,
        Some("rect") => 3,
        Some("text") => 4,
        Some("graphic") => 5,
        Some("model") => 6,
        Some("invtext") => 7,
        _ => -1,
    }
}

fn name_to_button_type(name: Option<&str>) -> i64 {
    match name {
        Some("normal") => 1,
        Some("target") => 2,
        Some("close") => 3,
        Some("toggle") => 4,
        Some("select") => 5,
        Some("pause") => 6,
        _ => 0,
    }
}

fn name_to_comparator(name: &str) -> i64 {
    match name {
        "eq" => 1,
        "lt" => 2,
        "gt" => 3,
        "neq" => 4,
        _ => 0,
    }
}

const SCRIPT_OPS: &[&str] = &[
    "", "stat_level", "stat_base_level", "stat_xp", "inv_count", "pushvar", "stat_xp_remaining", "op7", "op8", "op9", "inv_contains", "runenergy", "runweight",
    "testbit", "push_varbit", "subtract", "divide", "multiply", "coordx", "coordz", "push_constant",
];

fn name_to_script(name: &str) -> i64 {
    SCRIPT_OPS.iter().skip(1).position(|s| *s == name).map_or(0, |i| i as i64 + 1)
}

/// Extra operand words per script op (`opCount += n`).
fn script_operands(name: &str) -> usize {
    match name {
        "stat_level" | "stat_base_level" | "stat_xp" | "pushvar" | "stat_xp_remaining" | "push_varbit" | "push_constant" => 1,
        "inv_count" | "inv_contains" | "testbit" => 2,
        _ => 0,
    }
}

fn name_to_stat(name: Option<&str>) -> i64 {
    let stats = [
        "attack", "defence", "strength", "hitpoints", "ranged", "prayer", "magic", "cooking", "woodcutting", "fletching", "fishing", "firemaking", "crafting",
        "smithing", "mining", "herblore", "agility", "thieving",
    ];
    match name {
        Some("runecraft") => 20,
        Some(n) => stats.iter().position(|s| *s == n).map_or(-1, |i| i as i64),
        None => -1,
    }
}

fn name_to_font(name: Option<&str>) -> i64 {
    match name {
        Some("p11_full") => 0,
        Some("p12_full") => 1,
        Some("b12_full") => 2,
        Some("q8_full") => 3,
        _ => -1,
    }
}

/// `packInterface(modelFlags)`: `(client data, server interface.dat)`.
pub fn pack_interface(b: &Build, packs: &Packs, model_flags: &mut Vec<i32>) -> Result<(Vec<u8>, Vec<u8>), String> {
    let pack = &packs.interface;
    let order = load_order(&format!("{}/pack/interface.order", b.src_dir))?;
    let mut component: HashMap<i64, Com> = order.iter().map(|&id| (id, Com::default())).collect();

    for entry in load_dir(&format!("{}/scripts", b.src_dir), ".if") {
        let (src, file, dir) = entry?;
        let if_name = file.replacen(".if", "", 1);
        let Some((if_id, root)) = pack.get_by_name(&if_name).and_then(|id| Some((id, component.get_mut(&id)?))) else {
            return Err(format!("Could not find name <-> ID for interface file, perhaps misnamed? {if_name}"));
        };
        for (k, v) in [("type", "layer"), ("width", "512"), ("height", "334")] {
            root.src.insert(k.into(), (v.into(), String::new()));
        }

        let mut com_name = String::new();
        let mut com_id = None;
        for (n, line) in &src {
            if line.starts_with('[') {
                com_name = inner(line).to_string();
                let id = pack.get_by_name(&format!("{if_name}:{com_name}")).filter(|id| component.contains_key(id));
                let Some(id) = id else {
                    return Err(format!("Missing component ID {if_name}:{com_name} in {}/pack/interface.order", b.src_dir));
                };
                com_id = Some(id);
                component.get_mut(&id).unwrap().root = Some(if_name.clone());
                component.get_mut(&if_id).unwrap().children.push(id);
                continue;
            }
            // `line.substring(0, line.indexOf('='))`: no `=` gives key '' and the whole line as value.
            let (key, value) = match line.find('=') {
                Some(i) => (&line[..i], &line[i + 1..]),
                None => ("", line.as_str()),
            };
            if key == "layer" {
                let layer_id = pack.get_by_name(&format!("{if_name}:{value}")).ok_or_else(|| format!("ERROR: Layer {if_name}:{value} does not exist"))?;
                let layer = component.get_mut(&layer_id).ok_or_else(|| format!("layer {layer_id} is not in interface.order"))?;
                // before any `[component]` the id is -1 (no component has it)
                let child = com_id.unwrap_or(-1);
                if layer.children.contains(&child) {
                    return Err(format!("ERROR: Layer {if_name}:{value} already has {com_name} as a child"));
                }
                layer.children.push(child);
                // `splice(indexOf(comId), 1)`: an index of -1 removes the last child.
                let siblings = &mut component.get_mut(&if_id).unwrap().children;
                match siblings.iter().position(|&c| c == child) {
                    Some(i) => {
                        siblings.remove(i);
                    }
                    None => {
                        siblings.pop();
                    }
                }
            }
            let target = com_id.unwrap_or(if_id);
            component.get_mut(&target).unwrap().src.insert(key.to_string(), (value.to_string(), format!("{dir}/{file}:{n}")));
        }
    }

    let mut client = Packet::alloc(5);
    let mut server = Packet::alloc(5);
    let mut last_root: Option<String> = None;
    client.p2(pack.max);
    server.p2(pack.max);
    for &id in &order {
        let com = &component[&id];
        if com.root.is_none() || last_root != com.root {
            client.p2(-1);
            match &com.root {
                Some(root) => {
                    client.p2(pack.get_by_name(root).unwrap_or(-1));
                    last_root = Some(root.clone());
                }
                None => {
                    client.p2(id);
                    last_root = Some(pack.get_by_id(id).to_string());
                }
            }
        }
        client.p2(id);
        server.p2(id);
        server.pjstr(pack.get_by_id(id));
        server.pbool(com.get("type") == Some("overlay"));

        let com_type = name_to_type(com.get("type"));
        client.p1(com_type);
        let button_type = name_to_button_type(com.get("buttontype"));
        client.p1(button_type);
        client.p2(com.int("clientcode")?);
        client.p2(com.int("width")?);
        client.p2(com.int("height")?);
        client.p1(if com.truthy("trans") { com.int("trans")? } else { 0 });
        if com.truthy("overlayer") {
            let root = com.root.as_deref().unwrap_or("null");
            // a missing overlayer is id -1 (+ 0x100)
            client.p2(pack.get_by_name(&format!("{root}:{}", com.get("overlayer").unwrap())).unwrap_or(-1) + 0x100);
        } else {
            client.p1(0);
        }

        let comparators = (1..=5).filter(|j| com.get(&format!("script{j}")).is_some()).count();
        client.p1(comparators as i64);
        for j in 1..=comparators {
            let k = format!("script{j}");
            let s = com.get(&k).ok_or_else(|| format!("{} has no script{j}", pack.get_by_id(id)))?;
            let mut parts = s.split(',');
            client.p1(name_to_comparator(parts.next().unwrap()));
            client.p2(com.num(&k, parts.next())?);
        }

        let scripts = (1..=5).filter(|j| com.get(&format!("script{j}op1")).is_some()).count();
        client.p1(scripts as i64);
        for j in 1..=scripts {
            let mut op_count = 0;
            for k in 1..=20 {
                if let Some(op) = com.get(&format!("script{j}op{k}")) {
                    op_count += 1 + script_operands(op.split(',').next().unwrap());
                }
            }
            if com.get(&format!("script{j}op1")) == Some("") {
                client.p2(op_count as i64);
            } else {
                client.p2(op_count as i64 + 1);
            }
            for k in 1..=op_count {
                let key = format!("script{j}op{k}");
                let Some(op) = com.get(&key).filter(|o| !o.is_empty()) else { continue };
                let parts: Vec<&str> = op.split(',').collect();
                let part = |i: usize| parts.get(i).copied();
                client.p2(name_to_script(parts[0]));
                // a failed lookup is logged and written as -1
                let lookup = |p: &crate::pack::pack_file::PackFile, what: Option<&str>| {
                    what.and_then(|n| p.get_by_name(n)).unwrap_or_else(|| {
                        tracing::error!("{} invalid lookup {}", com.root.as_deref().unwrap_or("null"), what.unwrap_or(""));
                        -1
                    })
                };
                match parts[0] {
                    "stat_level" | "stat_base_level" | "stat_xp" | "stat_xp_remaining" => client.p2(name_to_stat(part(1))),
                    "inv_count" | "inv_contains" => {
                        client.p2(lookup(pack, part(1)));
                        client.p2(lookup(&packs.obj, part(2)));
                    }
                    "pushvar" => client.p2(lookup(&packs.varp, part(1))),
                    "testbit" => {
                        client.p2(lookup(&packs.varp, part(1)));
                        client.p2(com.num(&key, part(2))?);
                    }
                    "push_varbit" => client.p2(lookup(&packs.varbit, part(1))),
                    "push_constant" => client.p2(com.num(&key, part(1))?),
                    _ => {}
                }
            }
            if op_count > 0 {
                client.p2(0);
            }
        }

        let opt = |k: &str| com.get(k).unwrap_or("").to_string();
        // `margin=x,y`; absent or empty → 0
        let margin = |i: usize| com.num("margin", com.get("margin").filter(|m| !m.is_empty()).and_then(|m| m.split(',').nth(i)));

        if com_type == 0 {
            client.p2(com.int("scroll")?);
            client.pbool(com.yes("hide"));
            client.p2(com.children.len() as i64);
            for child in &com.children {
                client.p2(*child);
                let c = component.get(child).ok_or_else(|| format!("child {child} has no component"))?;
                client.p2(c.int("x")?);
                client.p2(c.int("y")?);
            }
        }
        if com_type == 2 {
            client.pbool(com.yes("draggable"));
            client.pbool(com.yes("interactable"));
            client.pbool(com.yes("usable"));
            client.pbool(com.yes("swappable"));
            client.p1(margin(0)?);
            client.p1(margin(1)?);
            for j in 1..=20 {
                let k = format!("slot{j}");
                match com.get(&k) {
                    // `sprite[:x,y]`
                    Some(slot) => {
                        client.pbool(true);
                        let mut parts = slot.split(':');
                        let sprite = parts.next().unwrap();
                        let mut o = parts.next().filter(|p| !p.is_empty()).unwrap_or("0,0").split(',');
                        client.p2(com.num(&k, o.next())?);
                        client.p2(com.num(&k, o.next())?);
                        client.pjstr(sprite);
                    }
                    None => client.pbool(false),
                }
            }
            for j in 1..=5 {
                client.pjstr(&opt(&format!("option{j}")));
            }
        }
        if com_type == 3 {
            client.pbool(com.yes("fill"));
        }
        if com_type == 4 {
            client.pbool(com.yes("center"));
            client.p1(name_to_font(com.get("font")));
            client.pbool(com.yes("shadowed"));
            client.pjstr(&opt("text"));
            client.pjstr(&opt("activetext"));
        }
        if com_type == 3 || com_type == 4 {
            for k in ["colour", "activecolour", "overcolour", "activeovercolour"] {
                client.p4(com.int(k)?);
            }
        }
        if com_type == 5 {
            client.pjstr(&opt("graphic"));
            client.pjstr(&opt("activegraphic"));
        }
        if com_type == 6 {
            for (key, is_model) in [("model", true), ("activemodel", true), ("anim", false), ("activeanim", false)] {
                if com.truthy(key) {
                    let name = com.get(key).unwrap();
                    let found = if is_model { packs.model.get_by_name(name) } else { packs.seq.get_by_name(name) };
                    let found = found.ok_or_else(|| format!("\nError packing interfaces\n{} Invalid {key}: {name}", com.root.as_deref().unwrap_or("null")))?;
                    client.p2(found + 0x100);
                    if is_model {
                        let i = found as usize;
                        if model_flags.len() <= i {
                            model_flags.resize(i + 1, 0);
                        }
                        model_flags[i] |= 0x2;
                    }
                } else {
                    client.p1(0);
                }
            }
            client.p2(com.int("zoom")?);
            client.p2(com.int("xan")?);
            client.p2(com.int("yan")?);
        }
        if com_type == 7 {
            client.pbool(com.yes("center"));
            client.p1(name_to_font(com.get("font")));
            client.pbool(com.yes("shadowed"));
            client.p4(com.int("colour")?);
            client.p2(margin(0)?);
            client.p2(margin(1)?);
            client.pbool(com.yes("interactable"));
            for j in 1..=5 {
                client.pjstr(&opt(&format!("option{j}")));
            }
        }
        if button_type == 2 || com_type == 2 {
            client.pjstr(&opt("actionverb"));
            client.pjstr(&opt("action"));
            let mut flags = 0;
            if com.truthy("actiontarget") {
                let targets: Vec<&str> = com.get("actiontarget").unwrap().split(',').collect();
                for (name, bit) in [("obj", 0x1), ("npc", 0x2), ("loc", 0x4), ("player", 0x8), ("heldobj", 0x10)] {
                    if targets.contains(&name) {
                        flags |= bit;
                    }
                }
            }
            client.p2(flags);
        }
        if matches!(button_type, 1 | 4 | 5 | 6) {
            client.pjstr(&opt("option"));
        }
    }
    Ok((client.written().to_vec(), server.written().to_vec()))
}

/// `packClientInterface(cache, modelFlags)` for a clean build.
pub fn pack_client_interface(b: &Build, packs: &Packs, out: &Path, cache: &mut FileStream, model_flags: &mut Vec<i32>) -> Result<(), String> {
    let (client, server) = pack_interface(b, packs, model_flags)?;
    let mut jag = Jagfile::new(true);
    jag.write("data", &client);
    let packed = jag.to_bytes();
    std::fs::create_dir_all(out.join("client")).map_err(|e| e.to_string())?;
    std::fs::write(out.join("client/interface"), &packed).map_err(|e| format!("client/interface: {e}"))?;
    std::fs::create_dir_all(out.join("server")).map_err(|e| e.to_string())?;
    std::fs::write(out.join("server/interface.dat"), &server).map_err(|e| format!("server/interface.dat: {e}"))?;
    if b.verify && !Packet::checkcrc(&packed, 0, packed.len(), 2135735991) {
        return Err("interface checksum mismatch!\nYou can disable this safety check by setting BUILD_VERIFY=false".into());
    }
    cache.write(0, 3, &packed, 0);
    Ok(())
}
