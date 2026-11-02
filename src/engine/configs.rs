//! Every config the server reads, loaded like `World.reload()` (+ the fonts
//! `World.start()` loads).

use crate::cache::config::{
    ConfigSet, category::CategoryType, component::Components, dbrow::{DbRowType, DbTableIndex}, dbtable::DbTableType, enum_type::EnumType, hitsplat::HitsplatType, hunt::HuntType, idk::IdkType,
    inv::InvType, loc::LocType, mesanim::MesanimType, npc::NpcType, obj::ObjType, param::ParamType, seq::{SeqType, load_anim_delays}, spotanim::SpotanimType,
    struct_type::StructType, var::{VarBitType, VarNpcType, VarObjType, VarPlayerType, VarSharedType},
};
use crate::cache::font::{FontType, load_fonts};
use crate::io::filestream::FileStream;
use std::path::Path;
use std::sync::Arc;

pub struct Configs {
    pub params: ConfigSet<ParamType>,
    pub varps: ConfigSet<VarPlayerType>,
    pub varbits: ConfigSet<VarBitType>,
    pub varns: ConfigSet<VarNpcType>,
    pub varss: ConfigSet<VarSharedType>,
    pub varos: ConfigSet<VarObjType>,
    pub objs: ConfigSet<ObjType>,
    pub locs: ConfigSet<LocType>,
    pub npcs: ConfigSet<NpcType>,
    pub idks: ConfigSet<IdkType>,
    pub seqs: ConfigSet<SeqType>,
    pub spotanims: ConfigSet<SpotanimType>,
    pub hitsplats: ConfigSet<HitsplatType>,
    pub categories: ConfigSet<CategoryType>,
    pub enums: ConfigSet<EnumType>,
    pub structs: ConfigSet<StructType>,
    pub invs: ConfigSet<InvType>,
    pub mesanims: ConfigSet<MesanimType>,
    pub dbtables: ConfigSet<DbTableType>,
    pub dbrows: ConfigSet<DbRowType>,
    pub dbindex: DbTableIndex,
    pub hunts: ConfigSet<HuntType>,
    pub components: Components,
    pub fonts: Vec<FontType>,
    /// `AnimFrame` delays, loaded once per process (kept across reloads).
    pub anim_delays: Arc<Vec<i32>>,
}

impl Configs {
    /// Loads `pack` (`data/pack`). `members` is `Environment.node.members`.
    pub fn load(pack: &Path, members: bool) -> Result<Self, String> {
        let mut cache = FileStream::open(pack, false).map_err(|e| format!("cache: {e}"))?;
        let anim_delays = Arc::new(load_anim_delays(&mut cache));
        Self::load_with_delays(pack, members, anim_delays)
    }

    /// A reload: the frame delays stay those of the first load.
    pub fn load_with_delays(pack: &Path, members: bool, anim_delays: Arc<Vec<i32>>) -> Result<Self, String> {
        let params = ConfigSet::<ParamType>::load(pack, "param.dat")?;
        let objs = ConfigSet::<ObjType>::load_objs(pack, members, &params)?;
        let dbtables = ConfigSet::<DbTableType>::load(pack, "dbtable.dat")?;
        let dbrows = ConfigSet::<DbRowType>::load(pack, "dbrow.dat")?;
        let dbindex = DbTableIndex::new(&dbtables, &dbrows);
        Ok(Configs {
            varps: ConfigSet::load_with_client(pack, "varp.dat")?,
            varbits: ConfigSet::load_with_client(pack, "varbit.dat")?,
            objs,
            locs: ConfigSet::load_with_client(pack, "loc.dat")?,
            npcs: ConfigSet::load_with_client(pack, "npc.dat")?,
            idks: ConfigSet::load_with_client(pack, "idk.dat")?,
            seqs: SeqType::load(pack, anim_delays.clone())?,
            spotanims: ConfigSet::load_with_client(pack, "spotanim.dat")?,
            hitsplats: ConfigSet::load(pack, "hitsplat.dat")?,
            categories: ConfigSet::load(pack, "category.dat")?,
            enums: ConfigSet::load(pack, "enum.dat")?,
            structs: ConfigSet::load(pack, "struct.dat")?,
            invs: ConfigSet::load(pack, "inv.dat")?,
            mesanims: ConfigSet::load(pack, "mesanim.dat")?,
            dbtables,
            dbrows,
            dbindex,
            hunts: ConfigSet::load(pack, "hunt.dat")?,
            varns: ConfigSet::load(pack, "varn.dat")?,
            varss: ConfigSet::load(pack, "vars.dat")?,
            varos: ConfigSet::load(pack, "varo.dat")?,
            components: Components::load(pack)?,
            fonts: load_fonts(pack)?,
            params,
            anim_delays,
        })
    }
}
