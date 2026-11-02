//! The config packers with a client half in the `config` jag: `SeqConfig`,
//! `FloConfig`, `SpotAnimConfig`, `IdkConfig`, `LocConfig`, `NpcConfig`,
//! `ObjConfig`. Index-addressed collections are sparse `Vec<Option<i64>>`:
//! holes write 0 unless another default is substituted (B.8).

use super::server::{Configs, each_config};
use super::shared::*;
use crate::cache::config::obj::get_wear_pos_id;
use crate::pack::parse::int;
use crate::util::color::rgb15_to_hsl16;

/// `recolNs` / `recolNd` (`retexN…` likewise), N a single digit 1-9:
/// `(N - 1, is_s)`.
fn recol(key: &str, prefix: &str) -> Option<(usize, bool)> {
    let (n, sd) = key.strip_prefix(prefix)?.split_at_checked(1)?;
    let n = n.parse::<usize>().ok().filter(|&n| n >= 1)?;
    match sd {
        "s" => Some((n - 1, true)),
        "d" => Some((n - 1, false)),
        _ => None,
    }
}

/// A recolour value: strict int, optionally through `rgb15toHsl16` (its maths are f64, B.9).
fn recol_value(key: &str, value: &str, to_hsl: bool) -> Parsed {
    recol(key, "recol").ok_or(Bad::Key)?;
    let n = int(value).ok_or(Bad::Value)?;
    Ok(Value::Num(if to_hsl { hsl(n) } else { n }))
}

/// `ColorConversion.rgb15toHsl16(n)`; the input wraps to i32 like `>>` does.
fn hsl(n: i64) -> i64 {
    rgb15_to_hsl16(n as i32) as i64
}

/// A key `<prefix>N` checked at parse time ([`key_index`]).
fn indexed(key: &str, prefixes: &[&str]) -> Result<(), Bad> {
    if prefixes.iter().any(|p| key_index(key, p).is_some()) { Ok(()) } else { Err(Bad::Key) }
}

/// A sparse slot: holes are `None`.
fn at(v: &[Option<i64>], i: usize) -> Option<i64> {
    v.get(i).copied().flatten()
}

// ---------------------------------------------------------------- seq

pub fn parse_seq_config(ctx: &Ctx, key: &str, value: &str) -> Parsed {
    let p = ctx.packs;
    match key {
        "loops" | "priority" | "maxloops" => parse_number(value),
        "reachforward" => parse_bool(value),
        _ if key.starts_with("frame") || key.starts_with("iframe") => {
            indexed(key, &["frame", "iframe"])?;
            pack_index(p.anim.get_by_name(value))
        }
        _ if key.starts_with("delay") => {
            indexed(key, &["delay"])?;
            parse_number(value)
        }
        // `label_N` or `N`
        "walkmerge" => value.split(',').map(|part| int(part.split_once('_').map_or(part, |(_, n)| n))).collect::<Option<_>>().map(Value::Nums).ok_or(Bad::Value),
        "replaceheldleft" | "replaceheldright" => {
            if value == "hide" {
                return Ok(Value::Num(0));
            }
            p.obj.get_by_name(value).map(|i| Value::Num(i + 512)).ok_or(Bad::Value)
        }
        "preanim_move" => match value {
            "delaymove" => Ok(Value::Num(0)),
            "delayanim" => Ok(Value::Num(1)),
            "merge" => Ok(Value::Num(2)),
            _ => Err(Bad::Value),
        },
        "postanim_move" => match value {
            "delaymove" => Ok(Value::Num(0)),
            "abortanim" => Ok(Value::Num(1)),
            "merge" => Ok(Value::Num(2)),
            _ => Err(Bad::Value),
        },
        "duplicatebehaviour" => match value {
            "0" => Ok(Value::Num(0)),
            "reset" => Ok(Value::Num(1)),
            "reset_loop" => Ok(Value::Num(2)),
            _ => Err(Bad::Value),
        },
        _ => Err(Bad::Key),
    }
}

pub fn pack_seq_configs(ctx: &Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    each_config(&ctx.packs.seq, configs, |_, config, client, _server| {
        let (mut frames, mut iframes, mut delays) = (Vec::new(), Vec::new(), Vec::new());
        for (key, value) in config {
            if let Some(i) = key_index(key, "frame") {
                set_at(&mut frames, i, value.num());
            } else if let Some(i) = key_index(key, "iframe") {
                set_at(&mut iframes, i, value.num());
            } else if let Some(i) = key_index(key, "delay") {
                set_at(&mut delays, i, value.num());
            } else {
                match key.as_str() {
                    "loops" => {
                        client.p1(2);
                        client.p2(value.num());
                    }
                    "walkmerge" => {
                        client.p1(3);
                        client.p1(value.nums().len() as i64);
                        for &l in value.nums() {
                            client.p1(l);
                        }
                    }
                    "reachforward" if value.is_true() => client.p1(4),
                    "priority" => {
                        client.p1(5);
                        client.p1(value.num());
                    }
                    "replaceheldleft" => {
                        client.p1(6);
                        client.p2(value.num());
                    }
                    "replaceheldright" => {
                        client.p1(7);
                        client.p2(value.num());
                    }
                    "maxloops" => {
                        client.p1(8);
                        client.p1(value.num());
                    }
                    "preanim_move" => {
                        client.p1(9);
                        client.p1(value.num());
                    }
                    "postanim_move" => {
                        client.p1(10);
                        client.p1(value.num());
                    }
                    "duplicatebehaviour" => {
                        client.p1(11);
                        client.p1(value.num());
                    }
                    _ => {}
                }
            }
        }
        if !frames.is_empty() {
            client.p1(1);
            client.p1(frames.len() as i64);
            for j in 0..frames.len() {
                client.p2(at(&frames, j).unwrap_or(0));
                client.p2(at(&iframes, j).unwrap_or(-1));
                client.p2(at(&delays, j).unwrap_or(0));
            }
        }
        Ok(())
    })
}

// ---------------------------------------------------------------- flo

pub fn parse_flo_config(ctx: &Ctx, key: &str, value: &str) -> Parsed {
    match key {
        "colour" => parse_number(value),
        "overlay" | "occlude" => parse_bool(value),
        "texture" => pack_index(ctx.packs.texture.get_by_name(value)),
        _ => Err(Bad::Key),
    }
}

pub fn pack_flo_configs(ctx: &Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    let pack = &ctx.packs.flo;
    let mut client = PackedData::new(pack.max);
    let mut server = PackedData::new(pack.max);
    for id in 0..pack.max {
        let debug_name = pack.get_by_id(id);
        if let Some(config) = configs.get(debug_name) {
            for (key, value) in config {
                match key.as_str() {
                    "colour" => {
                        client.p1(1);
                        client.p3(value.num());
                    }
                    "texture" => {
                        client.p1(2);
                        client.p1(value.num());
                    }
                    "overlay" if value.is_true() => client.p1(3),
                    "occlude" if value.is_false() => client.p1(5),
                    _ => {}
                }
            }
        }
        // "yes, this was originally transmitted!" — on the client, opcode 6, and never on the server.
        if !debug_name.is_empty() && !debug_name.starts_with("flo_") {
            client.p1(6);
            client.pjstr(debug_name);
        }
        client.next();
        server.next();
    }
    Ok((client, server))
}

// ---------------------------------------------------------------- spotanim

pub fn parse_spotanim_config(ctx: &Ctx, key: &str, value: &str) -> Parsed {
    let p = ctx.packs;
    match key {
        "resizeh" | "resizev" | "angle" | "ambient" | "contrast" => parse_number(value),
        "model" => pack_index(p.model.get_by_name(value)),
        "anim" => pack_index(p.seq.get_by_name(value)),
        _ if key.starts_with("recol") => recol_value(key, value, true),
        _ => Err(Bad::Key),
    }
}

pub fn pack_spotanim_configs(ctx: &mut Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    let mut flags = Vec::new();
    let r = each_config(&ctx.packs.spot_anim, configs, |_, config, client, _server| {
        for (key, value) in config {
            match key.as_str() {
                "model" => {
                    client.p1(1);
                    client.p2(value.num());
                    flags.push((value.num(), 0x2));
                }
                "anim" => {
                    client.p1(2);
                    client.p2(value.num());
                }
                "resizeh" => {
                    client.p1(4);
                    client.p2(value.num());
                }
                "resizev" => {
                    client.p1(5);
                    client.p2(value.num());
                }
                "angle" => {
                    client.p1(6);
                    client.p2(value.num());
                }
                "ambient" => {
                    client.p1(7);
                    client.p1(value.num());
                }
                "contrast" => {
                    client.p1(8);
                    client.p1(value.num());
                }
                _ if key.starts_with("recol") => {
                    let Some((i, s)) = recol(key, "recol") else { continue };
                    client.p1(if s { 40 } else { 50 } + i as i64);
                    client.p2(value.num());
                }
                _ => {}
            }
        }
        Ok(())
    });
    for (id, f) in flags {
        ctx.flag_model(id, f);
    }
    r
}

// ---------------------------------------------------------------- hitsplat

/// `sprite=<media sprite>,<index>`; `recolNs`/`recolNd` are RGB24 pixel values (no HSL conversion).
pub fn parse_hitsplat_config(_ctx: &Ctx, key: &str, value: &str) -> Parsed {
    match key {
        "sprite" => match value.split_once(',') {
            Some((name, i)) if !name.is_empty() && int(i).is_some_and(|i| (0..256).contains(&i)) => Ok(Value::Str(value.to_string())),
            _ => Err(Bad::Value),
        },
        _ if key.starts_with("recol") => recol_value(key, value, false),
        _ => Err(Bad::Key),
    }
}

pub fn pack_hitsplat_configs(ctx: &mut Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    each_config(&ctx.packs.hitsplat, configs, |_, config, client, _server| {
        for (key, value) in config {
            match key.as_str() {
                "sprite" => {
                    client.p1(1);
                    client.pjstr(value.str());
                }
                _ if key.starts_with("recol") => {
                    let Some((i, s)) = recol(key, "recol") else { continue };
                    client.p1(if s { 40 } else { 50 } + i as i64);
                    client.p3(value.num());
                }
                _ => {}
            }
        }
        Ok(())
    })
}

// ---------------------------------------------------------------- idk

const IDK_TYPES: &[&str] = &[
    "man_hair", "man_jaw", "man_torso", "man_arms", "man_hands", "man_legs", "man_feet", "woman_hair", "woman_jaw", "woman_torso", "woman_arms", "woman_hands",
    "woman_legs", "woman_feet",
];

pub fn parse_idk_config(ctx: &Ctx, key: &str, value: &str) -> Parsed {
    match key {
        "disable" => parse_bool(value),
        "type" => IDK_TYPES.iter().position(|t| *t == value).map(|i| Value::Num(i as i64)).ok_or(Bad::Value),
        _ if key.starts_with("model") || key.starts_with("head") => {
            indexed(key, &["model", "head"])?;
            pack_index(ctx.packs.model.get_by_name(value))
        }
        _ if key.starts_with("recol") => recol_value(key, value, true),
        _ => Err(Bad::Key),
    }
}

pub fn pack_idk_configs(ctx: &mut Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    let mut flags = Vec::new();
    let r = each_config(&ctx.packs.idk, configs, |_, config, client, _server| {
        let (mut recol_s, mut recol_d, mut models, mut heads) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (key, value) in config {
            if let Some(i) = key_index(key, "model") {
                set_at(&mut models, i, value.num());
                flags.push((value.num(), 0x80));
            } else if let Some(i) = key_index(key, "head") {
                set_at(&mut heads, i, value.num());
                flags.push((value.num(), 0x80));
            } else if let Some((i, s)) = recol(key, "recol") {
                set_at(if s { &mut recol_s } else { &mut recol_d }, i, value.num());
            } else if key == "type" {
                client.p1(1);
                client.p1(value.num());
            } else if key == "disable" && value.is_true() {
                client.p1(3);
            }
        }
        for (op, v) in [(40, &recol_s), (50, &recol_d), (60, &heads)] {
            for (i, x) in v.iter().enumerate() {
                client.p1(op + i as i64);
                client.p2(x.unwrap_or(0));
            }
        }
        if !models.is_empty() {
            client.p1(2);
            client.p1(models.len() as i64);
            for m in &models {
                client.p2(m.unwrap_or(0));
            }
        }
        Ok(())
    });
    for (id, f) in flags {
        ctx.flag_model(id, f);
    }
    r
}

// ---------------------------------------------------------------- loc

/// `LocShapeSuffix`: the map-editor key suffix for each shape (index = shape).
pub const LOC_SHAPE_SUFFIX: [&str; 23] = [
    "_1", "_2", "_3", "_4", "_q", "_w", "_r", "_e", "_t", "_5", "_8", "_9", "_a", "_s", "_d", "_f", "_g", "_h", "_z", "_x", "_c", "_v", "_0",
];
const CENTREPIECE_STRAIGHT: i64 = 10;

pub fn parse_loc_config(ctx: &Ctx, key: &str, value: &str) -> Parsed {
    let p = ctx.packs;
    match key {
        "name" | "desc" | "op1" | "op2" | "op3" | "op4" | "op5" | "model" | "model2" | "model3" | "model4" | "model5" => parse_string(value),
        "width" | "length" | "wallwidth" | "ambient" | "contrast" | "mapfunction" | "resizex" | "resizey" | "resizez" | "mapscene" | "offsetx" | "offsety"
        | "offsetz" => parse_number(value),
        "blockwalk" | "blockrange" | "active" | "hillskew" | "sharelight" | "occlude" | "mirror" | "shadow" | "forcedecor" | "breakroutefinding"
        | "raiseobject" => parse_bool(value),
        _ if key.starts_with("recol") => recol_value(key, value, true),
        _ if key.starts_with("retex") => {
            recol(key, "retex").ok_or(Bad::Key)?;
            pack_index(p.texture.get_by_name(value))
        }
        "category" => pack_index(p.category.get_by_name(value)),
        "anim" => pack_index(p.seq.get_by_name(value)),
        "param" => parse_param(ctx, value),
        "forceapproach" => Ok(Value::Num(match value {
            "north" => 0b1110,
            "east" => 0b1101,
            "south" => 0b1011,
            "west" => 0b0111,
            _ => 0b1111,
        })),
        _ => Err(Bad::Key),
    }
}

pub fn pack_loc_configs(ctx: &mut Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    let mut flags = Vec::new();
    let packs = ctx.packs;
    let r = each_config(&packs.loc, configs, |debug_name, config, client, server| {
        let (mut recol_s, mut recol_d) = (Vec::new(), Vec::new());
        let mut src_models: Vec<&str> = Vec::new();
        let mut name: Option<String> = None;
        let mut active = -1;
        let mut desc: Option<String> = None;
        let mut params: Vec<ParamValue> = Vec::new();

        for (key, value) in config {
            let k = key.as_str();
            if k == "name" {
                name = Some(value.str().to_string());
            } else if k == "desc" {
                desc = Some(value.str().to_string());
            } else if k.starts_with("model") {
                src_models.push(value.str());
            } else if let Some((i, s)) = recol(k, "recol").or_else(|| recol(k, "retex")) {
                // retextures are stored in recol until rev 465
                set_at(if s { &mut recol_s } else { &mut recol_d }, i, value.num());
            } else if k == "param" {
                if let Value::Param(p) = value {
                    params.push(p.clone());
                }
            } else if let Some(i) = key_index(k, "op") {
                client.p1(30 + i as i64);
                client.pjstr(value.str());
            } else {
                match k {
                    "width" => {
                        client.p1(14);
                        client.p1(value.num());
                    }
                    "length" => {
                        client.p1(15);
                        client.p1(value.num());
                    }
                    "blockwalk" if value.is_false() => client.p1(17),
                    "blockrange" if value.is_false() => client.p1(18),
                    "active" => {
                        client.p1(19);
                        client.pbool(value.is_true());
                        active = value.is_true() as i32;
                    }
                    "hillskew" if value.is_true() => client.p1(21),
                    "sharelight" if value.is_true() => client.p1(22),
                    "occlude" if value.is_true() => client.p1(23),
                    "anim" => {
                        client.p1(24);
                        client.p2(value.num());
                    }
                    "wallwidth" => {
                        client.p1(28);
                        client.p1(value.num());
                    }
                    "ambient" => {
                        client.p1(29);
                        client.p1(value.num());
                    }
                    "contrast" => {
                        client.p1(39);
                        client.p1(value.num());
                    }
                    "mapfunction" => {
                        client.p1(60);
                        client.p2(value.num());
                    }
                    "category" => {
                        server.p1(61);
                        server.p2(value.num());
                    }
                    "mirror" if value.is_true() => client.p1(62),
                    "shadow" if value.is_false() => client.p1(64),
                    "resizex" => {
                        client.p1(65);
                        client.p2(value.num());
                    }
                    "resizey" => {
                        client.p1(66);
                        client.p2(value.num());
                    }
                    "resizez" => {
                        client.p1(67);
                        client.p2(value.num());
                    }
                    "mapscene" => {
                        client.p1(68);
                        client.p2(value.num());
                    }
                    "forceapproach" => {
                        client.p1(69);
                        client.p1(value.num());
                    }
                    "offsetx" => {
                        client.p1(70);
                        client.p2(value.num());
                    }
                    "offsety" => {
                        client.p1(71);
                        client.p2(value.num());
                    }
                    "offsetz" => {
                        client.p1(72);
                        client.p2(value.num());
                    }
                    "forcedecor" if value.is_true() => client.p1(73),
                    "breakroutefinding" if value.is_true() => client.p1(74),
                    "raiseobject" => {
                        client.p1(75);
                        client.pbool(value.is_true());
                    }
                    _ => {}
                }
            }
        }

        if !recol_s.is_empty() {
            client.p1(40);
            client.p1(recol_s.len() as i64);
            for k in 0..recol_s.len() {
                client.p2(at(&recol_s, k).unwrap_or(0));
                client.p2(at(&recol_d, k).unwrap_or(0));
            }
        }

        let mut models: Vec<(i64, i64)> = Vec::new();
        for src in &src_models {
            // centrepiece_straight comes first in their data, so it is checked first
            if let Some(id) = packs.model.get_by_name(src) {
                flags.push((id, 0x4));
                models.push((id, CENTREPIECE_STRAIGHT));
            }
            for (shape, suffix) in LOC_SHAPE_SUFFIX.iter().enumerate() {
                if shape as i64 == CENTREPIECE_STRAIGHT {
                    continue;
                }
                if let Some(id) = packs.model.get_by_name(&format!("{src}{suffix}")) {
                    flags.push((id, 0x4));
                    models.push((id, shape as i64));
                }
            }
        }
        if !src_models.is_empty() && models.is_empty() {
            return Err(pack_step_error(debug_name, "Failed to find suitable loc models"));
        }
        // Stable sort: centrepiece_straight first, then by shape.
        models.sort_by(|a, b| {
            use std::cmp::Ordering;
            if a.1 == b.1 {
                Ordering::Equal
            } else if a.1 == CENTREPIECE_STRAIGHT {
                Ordering::Less
            } else if b.1 == CENTREPIECE_STRAIGHT {
                Ordering::Greater
            } else {
                a.1.cmp(&b.1)
            }
        });
        if !models.is_empty() {
            if models.iter().all(|m| m.1 == CENTREPIECE_STRAIGHT) {
                client.p1(5);
                client.p1(models.len() as i64);
                for m in &models {
                    client.p2(m.0);
                }
            } else {
                client.p1(1);
                client.p1(models.len() as i64);
                for m in &models {
                    client.p2(m.0);
                    client.p1(m.1);
                }
            }
        }

        if name.is_none() && active != 0 {
            // a loc with no name= but active=yes or any centrepiece_straight model transmits its debug_name
            let transmit = active == 1 || (active == -1 && models.iter().any(|m| m.1 == CENTREPIECE_STRAIGHT));
            if transmit {
                name = Some(debug_name.to_string());
            }
        }
        if let Some(name) = &name {
            client.p1(2);
            client.pjstr(name);
        }
        if let Some(desc) = &desc {
            client.p1(3);
            client.pjstr(desc);
        }
        if !params.is_empty() {
            write_params(server, &params);
        }
        Ok(())
    });
    for (id, f) in flags {
        ctx.flag_model(id, f);
    }
    r
}

// ---------------------------------------------------------------- npc

pub fn parse_npc_config(ctx: &Ctx, key: &str, value: &str) -> Parsed {
    let p = ctx.packs;
    match key {
        "name" | "desc" | "op1" | "op2" | "op3" | "op4" | "op5" => parse_string(value),
        "size" | "resizex" | "resizey" | "resizez" | "resizeh" | "resizev" | "wanderrange" | "maxrange" | "huntrange" | "attackrange" | "hitpoints" | "attack"
        | "strength" | "defence" | "magic" | "ranged" | "timer" | "respawnrate" | "ambient" | "contrast" | "headicon" | "turnspeed" | "regenrate" => parse_number(value),
        "minimap" | "members" | "givechase" | "alwaysontop" => parse_bool(value),
        _ if key.starts_with("model") || key.starts_with("head") => {
            indexed(key, &["model", "head"])?;
            pack_index(p.model.get_by_name(value))
        }
        _ if key.starts_with("recol") => recol_value(key, value, false),
        "readyanim" => pack_index(p.seq.get_by_name(value)),
        "walkanim" => {
            if value.contains(',') {
                let indices: Vec<i64> = value.split(',').map(|a| p.seq.get_by_name(a)).collect::<Option<_>>().ok_or(Bad::Value)?;
                return if indices.len() == 4 { Ok(Value::Nums(indices)) } else { Err(Bad::Value) };
            }
            pack_index(p.seq.get_by_name(value))
        }
        "vislevel" if value == "hide" => Ok(Value::Num(0)),
        "vislevel" => parse_number(value),
        "category" => pack_index(p.category.get_by_name(value)),
        "param" => parse_param(ctx, value),
        "moverestrict" => ["normal", "blocked", "blocked+normal", "indoors", "outdoors", "nomove", "passthru"]
            .iter()
            .position(|m| *m == value)
            .map(|i| Value::Num(i as i64))
            .ok_or(Bad::Value),
        "blockwalk" => match value {
            "none" => Ok(Value::Num(0)),
            "NPC" => Ok(Value::Num(1)),
            "all" => Ok(Value::Num(2)),
            _ => Err(Bad::Value),
        },
        "huntmode" => pack_index(p.hunt.get_by_name(value)),
        "defaultmode" => match value {
            "none" => Ok(Value::Num(0)),
            "wander" => Ok(Value::Num(1)),
            "patrol" => Ok(Value::Num(2)),
            _ => Err(Bad::Value),
        },
        // `coord[,delay]`; no delay is 0
        _ if key.starts_with("patrol") => {
            let (coord_text, delay) = match value.split_once(',') {
                Some((c, d)) => (c, int(d).ok_or(Bad::Value)?),
                None => (value, 0),
            };
            let (level, x, z) = coord(coord_text).ok_or(Bad::Value)?;
            Ok(Value::Nums(vec![z | (x << 14) | (level << 28), delay]))
        }
        _ => Err(Bad::Key),
    }
}

pub fn pack_npc_configs(ctx: &mut Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    let mut flags = Vec::new();
    let r = each_config(&ctx.packs.npc, configs, |debug_name, config, client, server| {
        let (mut recol_s, mut recol_d, mut models, mut heads) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut name: Option<String> = None;
        let mut params: Vec<ParamValue> = Vec::new();
        let mut patrol: Vec<&[i64]> = Vec::new();
        let mut vislevel = false;

        for (key, value) in config {
            let k = key.as_str();
            if k == "name" {
                name = Some(value.str().to_string());
            } else if let Some(i) = key_index(k, "model") {
                set_at(&mut models, i, value.num());
                flags.push((value.num(), 0x2));
            } else if let Some(i) = key_index(k, "head") {
                set_at(&mut heads, i, value.num());
                flags.push((value.num(), 0x2));
            } else if let Some((i, s)) = recol(k, "recol") {
                set_at(if s { &mut recol_s } else { &mut recol_d }, i, value.num());
            } else if k == "param" {
                if let Value::Param(p) = value {
                    params.push(p.clone());
                }
            } else if k == "desc" {
                client.p1(3);
                client.pjstr(value.str());
            } else if k == "size" {
                client.p1(12);
                client.p1(value.num());
            } else if k == "readyanim" {
                client.p1(13);
                client.p2(value.num());
            } else if k == "walkanim" {
                if let Value::Nums(v) = value {
                    client.p1(17);
                    for &a in v.iter().take(4) {
                        client.p2(a);
                    }
                } else {
                    client.p1(14);
                    client.p2(value.num());
                }
            } else if k == "category" {
                server.p1(18);
                server.p2(value.num());
            } else if k == "wanderrange" {
                server.p1(26);
                server.p2(value.num());
            } else if k == "maxrange" {
                server.p1(27);
                server.p2(value.num());
            } else if let Some(i) = key_index(k, "op") {
                client.p1(30 + i as i64);
                client.pjstr(value.str());
            } else {
                let (target, op, wide): (u8, i64, bool) = match k {
                    "attack" => (1, 74, true),
                    "defence" => (1, 75, true),
                    "strength" => (1, 76, true),
                    "hitpoints" => (1, 77, true),
                    "ranged" => (1, 78, true),
                    "magic" => (1, 79, true),
                    "resizex" => (0, 90, true),
                    "resizey" => (0, 91, true),
                    "resizez" => (0, 92, true),
                    "minimap" => {
                        if value.is_false() {
                            client.p1(93);
                        }
                        continue;
                    }
                    "vislevel" => {
                        vislevel = true;
                        (0, 95, true)
                    }
                    "resizeh" => (0, 97, true),
                    "resizev" => (0, 98, true),
                    "alwaysontop" => {
                        client.p1(99);
                        continue;
                    }
                    "ambient" => (0, 100, false),
                    "contrast" => (0, 101, false),
                    "headicon" => (0, 102, true),
                    "turnspeed" => (0, 103, true),
                    "huntrange" => (1, 202, false),
                    "timer" => (1, 203, true),
                    "respawnrate" => (1, 204, true),
                    "moverestrict" => (1, 206, false),
                    "attackrange" => (1, 207, true),
                    "blockwalk" => (1, 208, false),
                    "huntmode" => (1, 209, false),
                    "defaultmode" => (1, 210, false),
                    "members" => {
                        if value.is_true() {
                            server.p1(211);
                        }
                        continue;
                    }
                    "givechase" => {
                        if value.is_false() {
                            server.p1(213);
                        }
                        continue;
                    }
                    "regenrate" => (1, 214, true),
                    _ if k.starts_with("patrol") => {
                        patrol.push(value.nums());
                        continue;
                    }
                    _ => continue,
                };
                let out = if target == 0 { &mut *client } else { &mut *server };
                out.p1(op);
                if wide {
                    out.p2(value.num());
                } else {
                    out.p1(value.num());
                }
            }
        }

        if !recol_s.is_empty() {
            client.p1(40);
            client.p1(recol_s.len() as i64);
            for k in 0..recol_s.len() {
                // holes are 0 (and `rgb15toHsl16(0)` when converted)
                let (s, d) = (at(&recol_s, k).unwrap_or(0), at(&recol_d, k).unwrap_or(0));
                if s >= 100 || d >= 100 {
                    client.p2(hsl(s));
                    client.p2(hsl(d));
                } else {
                    client.p2(s);
                    client.p2(d);
                }
            }
        }

        client.p1(2);
        client.pjstr(name.as_deref().unwrap_or(debug_name));

        if !models.is_empty() {
            client.p1(1);
            client.p1(models.len() as i64);
            for m in &models {
                client.p2(m.unwrap_or(0));
            }
        }
        if !heads.is_empty() {
            client.p1(60);
            client.p1(heads.len() as i64);
            for h in &heads {
                client.p2(h.unwrap_or(0));
            }
        }
        if !vislevel {
            client.p1(95);
            client.p2(1);
        }
        if !patrol.is_empty() {
            server.p1(212);
            server.p1(patrol.len() as i64);
            for p in &patrol {
                server.p4(p[0]);
                server.p1(p[1]);
            }
        }
        if !params.is_empty() {
            write_params(server, &params);
        }
        Ok(())
    });
    for (id, f) in flags {
        ctx.flag_model(id, f);
    }
    r
}

// ---------------------------------------------------------------- obj

pub fn parse_obj_config(ctx: &Ctx, key: &str, value: &str) -> Parsed {
    let p = ctx.packs;
    match key {
        "name" | "desc" | "op1" | "op2" | "op3" | "op4" | "op5" | "iop1" | "iop2" | "iop3" | "iop4" | "iop5" => parse_string(value),
        "2dzoom" | "2dxan" | "2dyan" | "2dxof" | "2dyof" | "2dzan" | "cost" | "respawnrate" | "resizex" | "resizey" | "resizez" | "ambient" | "contrast" => {
            parse_number(value)
        }
        "stackable" | "members" | "tradeable" => parse_bool(value),
        "model" | "manwear2" | "womanwear2" | "manwear3" | "womanwear3" | "manhead" | "womanhead" | "manhead2" | "womanhead2" => {
            pack_index(p.model.get_by_name(value))
        }
        _ if key.starts_with("recol") => recol_value(key, value, false),
        "code10" => pack_index(p.seq.get_by_name(value)),
        "wearpos" | "wearpos2" | "wearpos3" => {
            let w = get_wear_pos_id(value);
            if w == -1 { Err(Bad::Value) } else { Ok(Value::Num(w as i64)) }
        }
        "manwear" | "womanwear" => {
            let (model, offset) = value.split_once(',').ok_or(Bad::Value)?;
            let model = p.model.get_by_name(model).ok_or(Bad::Value)?;
            Ok(Value::Nums(vec![model, int(offset).ok_or(Bad::Value)?]))
        }
        "weight" => {
            // an int with a unit; grams are f64 maths truncated toward zero (B.9)
            let (n, scale) = [("kg", 1000.0), ("oz", 28.3495), ("lb", 453.592), ("g", 1.0)]
                .into_iter()
                .find_map(|(u, scale)| value.strip_suffix(u).map(|n| (n, scale)))
                .ok_or(Bad::Value)?;
            let grams = int(n).ok_or(Bad::Value)? as f64 * scale;
            if !(-32768.0..=32767.0).contains(&grams) {
                return Err(Bad::Value);
            }
            Ok(Value::Num(grams as i64))
        }
        "dummyitem" => match value {
            "graphic_only" => Ok(Value::Num(1)),
            "inv_only" => Ok(Value::Num(2)),
            _ => Err(Bad::Value),
        },
        _ if key.starts_with("count") => {
            indexed(key, &["count"])?;
            let (obj, count) = value.split_once(',').ok_or(Bad::Value)?;
            let obj = p.obj.get_by_name(obj).ok_or(Bad::Value)?;
            let count = int(count).filter(|c| (1..=65535).contains(c)).ok_or(Bad::Value)?;
            Ok(Value::Nums(vec![obj, count]))
        }
        "category" => pack_index(p.category.get_by_name(value)),
        "param" => parse_param(ctx, value),
        "certlink" | "certtemplate" => pack_index(p.obj.get_by_name(value)),
        _ => Err(Bad::Key),
    }
}

pub fn pack_obj_configs(ctx: &mut Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    let packs = ctx.packs;
    let pack = &packs.obj;
    // a missing template is written as id -1
    let template_for_cert = pack.get_by_name("template_for_cert").unwrap_or_else(|| {
        tracing::warn!("necessary template_for_cert does not exist");
        -1
    });
    let mut flags: Vec<(i64, i32)> = Vec::new();
    let mut client = PackedData::new(pack.max);
    let mut server = PackedData::new(pack.max);

    for id in 0..pack.max {
        let debug_name = pack.get_by_id(id).to_string();
        let config: Option<ConfigLines> = if let Some(uncert_name) = debug_name.strip_prefix("cert_") {
            let uncert = pack.get_by_name(uncert_name).ok_or_else(|| pack_step_error(&debug_name, "Cert does not link to anything based on its name."))?;
            Some(vec![("certlink".into(), Value::Num(uncert)), ("certtemplate".into(), Value::Num(template_for_cert))])
        } else {
            configs.get(&debug_name).map(|config| {
                let mut config = config.clone();
                let has_name = config.iter().any(|(k, _)| k == "name");
                let has_model = config.iter().any(|(k, _)| k == "model");
                if !has_name && has_model {
                    // `debug_name.charAt(0).toUpperCase() + debug_name.slice(1).replace(/_/g, ' ')`
                    let mut units = debug_name.encode_utf16();
                    let first = units.next().map(|u| String::from_utf16_lossy(&[u]).to_uppercase()).unwrap_or_default();
                    let rest = String::from_utf16_lossy(&units.collect::<Vec<_>>()).replace('_', " ");
                    config.push(("name".into(), Value::Str(first + &rest)));
                }
                config
            })
        };

        if let Some(config) = config {
            let (mut recol_s, mut recol_d) = (Vec::new(), Vec::new());
            let mut name: Option<String> = None;
            let mut params: Vec<ParamValue> = Vec::new();
            let (mut model, mut worn): (Vec<i64>, Vec<i64>) = (Vec::new(), Vec::new());
            let mut members = false;

            for (key, value) in &config {
                let k = key.as_str();
                let v = value.num();
                if k == "name" {
                    name = Some(value.str().to_string());
                } else if let Some((i, s)) = recol(k, "recol") {
                    set_at(if s { &mut recol_s } else { &mut recol_d }, i, v);
                } else if k == "param" {
                    if let Value::Param(p) = value {
                        params.push(p.clone());
                    }
                } else if let Some(i) = key_index(k, "op") {
                    client.p1(30 + i as i64);
                    client.pjstr(value.str());
                } else if let Some(i) = key_index(k, "iop") {
                    client.p1(35 + i as i64);
                    client.pjstr(value.str());
                } else if let Some(i) = key_index(k, "count") {
                    client.p1(100 + i as i64);
                    client.p2(value.nums()[0]);
                    client.p2(value.nums()[1]);
                } else {
                    match k {
                        "model" => {
                            client.p1(1);
                            client.p2(v);
                            model.push(v);
                        }
                        "desc" => {
                            client.p1(3);
                            client.pjstr(value.str());
                        }
                        "2dzoom" => {
                            client.p1(4);
                            client.p2(v);
                        }
                        "2dxan" => {
                            client.p1(5);
                            client.p2(v);
                        }
                        "2dyan" => {
                            client.p1(6);
                            client.p2(v);
                        }
                        "2dxof" => {
                            client.p1(7);
                            client.p2(v);
                        }
                        "2dyof" => {
                            client.p1(8);
                            client.p2(v);
                        }
                        "code10" => {
                            client.p1(10);
                            client.p2(v);
                        }
                        "stackable" if value.is_true() => client.p1(11),
                        "cost" => {
                            client.p1(12);
                            client.p4(v);
                        }
                        "wearpos" => {
                            server.p1(13);
                            server.p1(v);
                        }
                        "wearpos2" => {
                            server.p1(14);
                            server.p1(v);
                        }
                        "tradeable" if value.is_false() => server.p1(15),
                        "members" if value.is_true() => {
                            client.p1(16);
                            members = true;
                        }
                        "manwear" | "womanwear" => {
                            let n = value.nums();
                            client.p1(if k == "manwear" { 23 } else { 25 });
                            client.p2(n[0]);
                            client.p1(n[1]);
                            worn.push(n[0]);
                        }
                        "manwear2" | "womanwear2" => {
                            client.p1(if k == "manwear2" { 24 } else { 26 });
                            client.p2(v);
                            worn.push(v);
                        }
                        "wearpos3" => {
                            server.p1(27);
                            server.p1(v);
                        }
                        "weight" => {
                            server.p1(75);
                            server.p2(v);
                        }
                        "manwear3" | "womanwear3" => {
                            client.p1(if k == "manwear3" { 78 } else { 79 });
                            client.p2(v);
                            worn.push(v);
                        }
                        "manhead" | "womanhead" | "manhead2" | "womanhead2" => {
                            client.p1(match k {
                                "manhead" => 90,
                                "womanhead" => 91,
                                "manhead2" => 92,
                                _ => 93,
                            });
                            client.p2(v);
                            flags.push((v, 0x80));
                        }
                        "category" => {
                            server.p1(94);
                            server.p2(v);
                        }
                        "2dzan" => {
                            client.p1(95);
                            client.p2(v);
                        }
                        "dummyitem" => {
                            server.p1(96);
                            server.p1(v);
                        }
                        "certlink" => {
                            client.p1(97);
                            client.p2(v);
                        }
                        "certtemplate" => {
                            client.p1(98);
                            client.p2(v);
                        }
                        "resizex" => {
                            client.p1(110);
                            client.p2(v);
                        }
                        "resizey" => {
                            client.p1(111);
                            client.p2(v);
                        }
                        "resizez" => {
                            client.p1(112);
                            client.p2(v);
                        }
                        "ambient" => {
                            client.p1(113);
                            client.p1(v);
                        }
                        "contrast" => {
                            client.p1(114);
                            client.p1(v);
                        }
                        "respawnrate" => {
                            server.p1(201);
                            server.p2(v);
                        }
                        _ => {}
                    }
                }
            }

            let (model_flag, worn_flag) = if members { (0x40, 0x10) } else { (0x20, 0x08) };
            flags.extend(model.iter().map(|&m| (m, model_flag)));
            flags.extend(worn.iter().map(|&w| (w, worn_flag)));

            // reverse-lookup the certificate (so the server can find it quicker)
            if let Some(cert) = pack.get_by_name(&format!("cert_{debug_name}")) {
                server.p1(97);
                server.p2(cert);
            }

            if !recol_s.is_empty() {
                client.p1(40);
                client.p1(recol_s.len() as i64);
                for k in 0..recol_s.len() {
                    let (s, d) = (at(&recol_s, k).unwrap_or(0), at(&recol_d, k).unwrap_or(0));
                    if s >= 100 || d >= 100 {
                        client.p2(hsl(s));
                        client.p2(hsl(d));
                    } else {
                        client.p2(s);
                        client.p2(d);
                    }
                }
            }
            if let Some(name) = &name {
                client.p1(2);
                client.pjstr(name);
            }
            if !params.is_empty() {
                write_params(&mut server, &params);
            }
        }

        if !debug_name.is_empty() {
            server.p1(250);
            server.pjstr(&debug_name);
        }
        client.next();
        server.next();
    }
    for (id, f) in flags {
        ctx.flag_model(id, f);
    }
    Ok((client, server))
}
