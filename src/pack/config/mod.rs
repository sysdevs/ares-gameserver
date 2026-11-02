//! `tools/pack/config/**`: `packConfigs()` for a clean build.
//!
//! Order (it fixes the `config` jag entry order and which decoders are
//! available to later steps): `category.dat`; constants; the global var-name
//! uniqueness check; param; reload ParamType; dbtable, reload DbTableType,
//! dbrow; enum, inv, mesanim, struct; then the client-visible seq, loc, flo,
//! spotanim, npc, obj, idk, varp, varbit, hitsplat (each `.dat`/`.idx` into
//! the jag, CRC-checked when `verify` is on, except hitsplat, which the
//! original client lacks); hunt, varn, vars; save the jag and put it in cache
//! archive 0 file 2.

pub mod client;
pub mod hunt;
pub mod server;
pub mod shared;

use crate::cache::config::{ConfigSet, dbtable::DbTableType, param::ParamType};
use crate::io::filestream::FileStream;
use crate::io::jagfile::Jagfile;
use crate::io::packet::Packet;
use crate::pack::pack_file::Packs;
use crate::pack::parse::list_dir;
use crate::util::config::Build;
use server::Configs;
use shared::{Ctx, PackedData, Parsed, read_configs};
use std::collections::HashSet;
use std::path::Path;

type Pack = fn(&mut Ctx, &Configs) -> Result<(PackedData, PackedData), String>;

/// One `readConfigs(...)` step.
struct Step {
    ext: &'static str,
    required: &'static [&'static str],
    parse: fn(&Ctx, &str, &str) -> Parsed,
    pack: Pack,
    /// `Some(crc)`: client-visible, written into the jag and CRC-checked.
    client_crc: Option<i32>,
    /// Client-visible with no CRC to check against (a config the original client lacks).
    client_unchecked: bool,
}

const fn step(ext: &'static str, parse: fn(&Ctx, &str, &str) -> Parsed, pack: Pack, client_crc: Option<i32>) -> Step {
    Step { ext, required: &[], parse, pack, client_crc, client_unchecked: false }
}

fn save(out: &Path, name: &str, p: &PackedData) -> Result<(), String> {
    let path = out.join("server").join(name);
    p.dat.save(path.with_extension("dat")).map_err(|e| format!("{name}.dat: {e}"))?;
    p.idx.save(path.with_extension("idx")).map_err(|e| format!("{name}.idx: {e}"))
}

fn run(ctx: &mut Ctx, files: &[String], s: &Step, out: &Path, jag: &mut Jagfile, b: &Build) -> Result<(), String> {
    let configs = read_configs(ctx, files, s.ext, s.required, s.parse)?;
    let (client, server) = (s.pack)(ctx, &configs)?;
    let name = &s.ext[1..];
    if s.client_crc.is_some() || s.client_unchecked {
        if let Some(crc) = s.client_crc
            && b.verify
            && !Packet::checkcrc(&client.dat.data, 0, client.dat.pos, crc)
        {
            return Err(format!("{} checksum mismatch!\nYou can disable this safety check by setting BUILD_VERIFY=false", s.ext));
        }
        jag.write(&format!("{name}.dat"), client.dat.written());
        jag.write(&format!("{name}.idx"), client.idx.written());
    }
    save(out, name, &server)
}

/// `packConfigs(cache, modelFlags)` writing into `out` (`data/pack`).
pub fn pack_configs(b: &Build, packs: &Packs, out: &Path, cache: &mut FileStream, model_flags: Vec<i32>) -> Result<Vec<i32>, String> {
    // category.dat
    let mut dat = Packet::alloc(1);
    dat.p2(packs.category.size() as i64);
    for i in 0..packs.category.size() {
        dat.p1(1);
        dat.pjstr(packs.category.get_by_id(i as i64));
        dat.p1(0);
    }
    dat.save(out.join("server/category.dat")).map_err(|e| format!("category.dat: {e}"))?;

    let constants = shared::load_constants(&b.src_dir)?;

    // var domains are global, so names must be unique across them
    let mut names = HashSet::new();
    for pack in [&packs.varp, &packs.varbit, &packs.varn, &packs.vars, &packs.varo] {
        for name in pack.names() {
            if !names.insert(name.clone()) {
                return Err(format!("Non-unique var name found: {name}"));
            }
        }
    }

    let files: Vec<String> = list_dir(&format!("{}/scripts", b.src_dir)).into_iter().filter(|f| !f.ends_with('/')).collect();
    let mut ctx = Ctx { packs, params: ConfigSet::default(), dbtables: ConfigSet::default(), constants, model_flags };
    let mut jag = Jagfile::new(false);

    let param = Step { ext: ".param", required: &["type"], parse: server::parse_param_config, pack: |c, cf| server::pack_param_configs(c, cf), client_crc: None, client_unchecked: false };
    run(&mut ctx, &files, &param, out, &mut jag, b)?;
    ctx.params = ConfigSet::<ParamType>::load(out, "param.dat")?;

    run(&mut ctx, &files, &step(".dbtable", server::parse_dbtable_config, |c, cf| server::pack_dbtable_configs(c, cf), None), out, &mut jag, b)?;
    ctx.dbtables = ConfigSet::<DbTableType>::load(out, "dbtable.dat")?;

    let steps = [
        step(".dbrow", server::parse_dbrow_config, |c, cf| server::pack_dbrow_configs(c, cf), None),
        step(".enum", server::parse_enum_config, |c, cf| server::pack_enum_configs(c, cf), None),
        step(".inv", server::parse_inv_config, |c, cf| server::pack_inv_configs(c, cf), None),
        step(".mesanim", server::parse_mesanim_config, |c, cf| server::pack_mesanim_configs(c, cf), None),
        step(".struct", server::parse_struct_config, |c, cf| server::pack_struct_configs(c, cf), None),
        step(".seq", client::parse_seq_config, |c, cf| client::pack_seq_configs(c, cf), Some(-753410077)),
        step(".loc", client::parse_loc_config, client::pack_loc_configs, Some(452815002)),
        step(".flo", client::parse_flo_config, |c, cf| client::pack_flo_configs(c, cf), Some(960212554)),
        step(".spotanim", client::parse_spotanim_config, client::pack_spotanim_configs, Some(-1587698939)),
        step(".npc", client::parse_npc_config, client::pack_npc_configs, Some(-1249602232)),
        step(".obj", client::parse_obj_config, client::pack_obj_configs, Some(128627047)),
        step(".idk", client::parse_idk_config, client::pack_idk_configs, Some(-359342366)),
        step(".varp", server::parse_varp_config, |c, cf| server::pack_varp_configs(c, cf), Some(703279713)),
        Step {
            ext: ".varbit",
            required: &["basevar", "startbit", "endbit"],
            parse: server::parse_varbit_config,
            pack: |c, cf| server::pack_varbit_configs(c, cf),
            client_crc: Some(-234977015),
            client_unchecked: false,
        },
        Step { client_unchecked: true, ..step(".hitsplat", client::parse_hitsplat_config, client::pack_hitsplat_configs, None) },
        step(".hunt", hunt::parse_hunt_config, |c, cf| hunt::pack_hunt_configs(c, cf), None),
        step(".varn", server::parse_var_type_config, |c, cf| server::pack_var_type_configs(&c.packs.varn, cf), None),
        step(".vars", server::parse_var_type_config, |c, cf| server::pack_var_type_configs(&c.packs.vars, cf), None),
        step(".varo", server::parse_var_type_config, |c, cf| server::pack_varo_configs(&c.packs.varo, cf), None),
    ];
    for s in &steps {
        run(&mut ctx, &files, s, out, &mut jag, b)?;
    }

    let config = jag.to_bytes();
    let path = out.join("client/config");
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, &config).map_err(|e| format!("client/config: {e}"))?;
    cache.write(0, 2, &config, 0);
    Ok(ctx.model_flags)
}
