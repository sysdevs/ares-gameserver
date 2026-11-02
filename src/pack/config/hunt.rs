//! The hunt config (server-only). Keeps the table bug `opobj2` → `OPOBJ1`
//! (deliberate: changes pack output).

use super::server::{Configs, each_config};
use super::shared::*;
use crate::pack::parse::int;

/// `HuntModeType`, `HuntVis`, `HuntCheckNotTooStrong`, `HuntNobodyNear`.
const MODE_OFF: i64 = 0;
const MODE_PLAYER: i64 = 1;
const MODE_NPC: i64 = 2;
const MODE_OBJ: i64 = 3;
const MODE_SCENERY: i64 = 4;
const NOBODYNEAR_PAUSEHUNT: i64 = 1;

/// `find_newmode` values → `NpcMode` (exact names).
fn npc_mode(value: &str) -> Option<i32> {
    if value == "opobj2" {
        return Some(27); // OPOBJ1, not OPOBJ2 (deliberate bug)
    }
    let families: [(&str, i32, i32); 9] =
        [("opplayer", 7, 5), ("applayer", 12, 5), ("queue", 47, 20), ("opobj", 27, 5), ("apobj", 32, 5), ("opnpc", 37, 5), ("apnpc", 42, 5), ("oploc", 17, 5), ("aploc", 22, 5)];
    families.iter().find_map(|&(prefix, base, count)| (1..=count).find(|i| value == format!("{prefix}{i}")).map(|i| base + i - 1))
}

fn on_off(value: &str) -> Parsed {
    match value {
        "off" => Ok(Value::Bool(false)),
        "on" => Ok(Value::Bool(true)),
        _ => Err(Bad::Value),
    }
}

/// `check_inv` / `check_invparam` / `check_invcat`: `inv,X,<op><val>`.
fn inv_check(value: &str, other: impl Fn(&str) -> Option<i64>, ctx: &Ctx) -> Parsed {
    let parts: Vec<&str> = value.split(',').collect();
    if parts.len() != 3 {
        return Err(Bad::Value);
    }
    let inv = ctx.packs.inv.get_by_name(parts[0]).ok_or(Bad::Value)?;
    let o = other(parts[1]).ok_or(Bad::Value)?;
    let (condition, val) = condition_val(parts[2], &['=', '>', '<', '!'])?;
    Ok(Value::HuntInv { inv, other: o, condition, val })
}

/// A condition character from `allowed` followed by a strict int (`>3`).
fn condition_val(s: &str, allowed: &[char]) -> Result<(String, i64), Bad> {
    let c = s.chars().next().filter(|c| allowed.contains(c)).ok_or(Bad::Value)?;
    let val = int(&s[c.len_utf8()..]).ok_or(Bad::Value)?;
    Ok((c.to_string(), val))
}

pub fn parse_hunt_config(ctx: &Ctx, key: &str, value: &str) -> Parsed {
    let p = ctx.packs;
    match key {
        "rate" => parse_number(value),
        "type" => ["off", "player", "npc", "obj", "scenery"].iter().position(|m| *m == value).map(|i| Value::Num(i as i64)).ok_or(Bad::Value),
        "check_vis" => ["off", "lineofsight", "lineofwalk"].iter().position(|m| *m == value).map(|i| Value::Num(i as i64)).ok_or(Bad::Value),
        "check_nottoostrong" => ["off", "outside_wilderness"].iter().position(|m| *m == value).map(|i| Value::Num(i as i64)).ok_or(Bad::Value),
        "check_notcombat" => pack_index(value.strip_prefix('%').and_then(|v| p.varp.get_by_name(v))),
        "check_notcombat_self" => pack_index(value.strip_prefix('%').and_then(|v| p.varn.get_by_name(v))),
        "check_notbusy" | "find_keephunting" | "check_afk" => on_off(value),
        "find_newmode" => npc_mode(value).map(|m| Value::Num(m as i64)).ok_or(Bad::Value),
        "nobodynear" => ["keephunting", "pausehunt"].iter().position(|m| *m == value).map(|i| Value::Num(i as i64)).ok_or(Bad::Value),
        "check_category" => pack_index(p.category.get_by_name(value)),
        "check_npc" => pack_index(p.npc.get_by_name(value)),
        "check_obj" => pack_index(p.obj.get_by_name(value)),
        "check_loc" => pack_index(p.loc.get_by_name(value)),
        "check_inv" => inv_check(value, |s| p.obj.get_by_name(s), ctx),
        "check_invparam" => inv_check(value, |s| p.param.get_by_name(s), ctx),
        "check_invcat" => inv_check(value, |s| p.category.get_by_name(s), ctx),
        "extracheck_var" => {
            let parts: Vec<&str> = value.split(',').collect();
            if parts.len() != 2 || !parts[0].starts_with('%') {
                return Err(Bad::Value);
            }
            // the condition character is checked before the varp lookup
            let first = parts[1].chars().next();
            if !first.is_some_and(|c| ['=', '>', '<', '!', '&'].contains(&c)) {
                return Err(Bad::Value);
            }
            let varp = p.varp.get_by_name(&parts[0][1..]).ok_or(Bad::Value)?;
            let (condition, val) = condition_val(parts[1], &['=', '>', '<', '!', '&'])?;
            Ok(Value::HuntVar { varp, condition, val })
        }
        _ => Err(Bad::Key),
    }
}

const ALL_CHECKS: [&str; 7] = ["check_category", "check_npc", "check_obj", "check_loc", "check_inv", "check_invparam", "check_invcat"];

pub fn pack_hunt_configs(ctx: &Ctx, configs: &Configs) -> Result<(PackedData, PackedData), String> {
    each_config(&ctx.packs.hunt, configs, |debug_name, config, _client, server| {
        let mut extracheck_vars = 0;
        // `config.every(x => x.key !== <every check but this one>)`
        let alone = |me: &str| config.iter().all(|(k, _)| k == me || !ALL_CHECKS.contains(&k.as_str()));
        let has_type = |modes: &[i64]| config.iter().any(|(k, v)| k == "type" && modes.contains(&v.num()));
        let unpackable = |key: &str| format!("Hunt config: [{debug_name}] unable to pack line!!!.\nInvalid property value: {key}=");

        for (key, value) in config {
            let v = value.num();
            match key.as_str() {
                "type" if v != MODE_OFF => {
                    server.p1(1);
                    server.p1(v);
                }
                "check_vis" if v != 0 => {
                    server.p1(2);
                    server.p1(v);
                }
                "check_nottoostrong" if v != 0 => {
                    server.p1(3);
                    server.p1(v);
                }
                "check_notbusy" if !value.is_false() => server.p1(4),
                "find_keephunting" if !value.is_false() => server.p1(5),
                "find_newmode" if v != 0 => {
                    server.p1(6);
                    server.p1(v);
                }
                "nobodynear" if v != NOBODYNEAR_PAUSEHUNT => {
                    server.p1(7);
                    server.p1(v);
                }
                "check_notcombat" => {
                    server.p1(8);
                    server.p2(v);
                }
                "check_notcombat_self" => {
                    server.p1(9);
                    server.p2(v);
                }
                "check_afk" if !value.is_true() => server.p1(10),
                "rate" if v != 1 => {
                    server.p1(11);
                    server.p2(v);
                }
                "check_category" | "check_npc" | "check_obj" | "check_loc" => {
                    let (op, modes): (i64, &[i64]) = match key.as_str() {
                        "check_category" => (12, &[MODE_NPC, MODE_OBJ, MODE_SCENERY]),
                        "check_npc" => (13, &[MODE_NPC]),
                        "check_obj" => (14, &[MODE_OBJ]),
                        _ => (15, &[MODE_SCENERY]),
                    };
                    if !(alone(key) && has_type(modes)) {
                        return Err(unpackable(key));
                    }
                    server.p1(op);
                    server.p2(v);
                }
                "check_inv" | "check_invparam" | "check_invcat" => {
                    if !(alone(key) && has_type(&[MODE_PLAYER])) {
                        return Err(unpackable(key));
                    }
                    let &Value::HuntInv { inv, other, ref condition, val } = value else { unreachable!() };
                    server.p1(match key.as_str() {
                        "check_inv" => 16,
                        "check_invparam" => 17,
                        _ => 18,
                    });
                    server.p2(inv);
                    server.p2(other);
                    server.pjstr(condition);
                    server.p4(val);
                }
                "extracheck_var" => {
                    if extracheck_vars > 2 {
                        return Err(format!("Hunt config: [{debug_name}] unable to pack line!!!.\nLimit of 3 extracheck_var properties exceeded."));
                    }
                    if !has_type(&[MODE_PLAYER]) {
                        return Err(unpackable(key));
                    }
                    let &Value::HuntVar { varp, ref condition, val } = value else { unreachable!() };
                    server.p1(19 + extracheck_vars);
                    server.p2(varp);
                    server.pjstr(condition);
                    server.p4(val);
                    extracheck_vars += 1;
                }
                _ => {}
            }
        }
        Ok(())
    })
}
