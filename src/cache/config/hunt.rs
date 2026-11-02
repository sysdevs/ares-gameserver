//! Hunt config (server-only). The hunt enums
//! (`HuntModeType`, `HuntVis`, ...) are plain ints here.

use super::ConfigType;
use crate::io::packet::Packet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct HuntVar {
    pub var_id: i32,
    pub condition: String,
    pub val: i32,
}

#[derive(Clone, Debug, Serialize)]
pub struct HuntType {
    pub id: usize,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: i32,
    pub check_vis: i32,
    pub check_not_too_strong: i32,
    pub check_not_busy: bool,
    pub find_keep_hunting: bool,
    pub find_new_mode: i32,
    pub nobody_near: i32,
    pub check_not_combat: i32,
    pub check_not_combat_self: i32,
    pub check_afk: bool,
    pub rate: i32,
    pub check_category: i32,
    pub check_npc: i32,
    pub check_obj: i32,
    pub check_loc: i32,
    pub check_inv: i32,
    pub check_obj_param: i32,
    pub check_obj_cat: i32,
    pub check_inv_condition: String,
    pub check_inv_val: i32,
    pub check_vars: Vec<HuntVar>,
}

impl ConfigType for HuntType {
    fn new(id: usize) -> Self {
        HuntType {
            id,
            name: None,
            kind: 0,
            check_vis: 0,
            check_not_too_strong: 0,
            check_not_busy: false,
            find_keep_hunting: false,
            find_new_mode: 0,
            nobody_near: 1, // PAUSEHUNT
            check_not_combat: -1,
            check_not_combat_self: -1,
            check_afk: true,
            rate: 1,
            check_category: -1,
            check_npc: -1,
            check_obj: -1,
            check_loc: -1,
            check_inv: -1,
            check_obj_param: -1,
            check_obj_cat: -1,
            check_inv_condition: String::new(),
            check_inv_val: -1,
            check_vars: Vec::new(),
        }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => self.kind = dat.g1(),
            2 => self.check_vis = dat.g1(),
            3 => self.check_not_too_strong = dat.g1(),
            4 => self.check_not_busy = true,
            5 => self.find_keep_hunting = true,
            6 => self.find_new_mode = dat.g1(),
            7 => self.nobody_near = dat.g1(),
            8 => self.check_not_combat = dat.g2(),
            9 => self.check_not_combat_self = dat.g2(),
            10 => self.check_afk = false,
            11 => self.rate = dat.g2(),
            12 => self.check_category = dat.g2(),
            13 => self.check_npc = dat.g2(),
            14 => self.check_obj = dat.g2(),
            15 => self.check_loc = dat.g2(),
            16..=18 => {
                self.check_inv = dat.g2();
                let v = dat.g2();
                match code {
                    16 => self.check_obj = v,
                    17 => self.check_obj_param = v,
                    _ => self.check_obj_cat = v,
                }
                self.check_inv_condition = dat.gjstr(10);
                self.check_inv_val = dat.g4s();
            }
            19..=21 => {
                let var_id = dat.g2();
                let condition = dat.gjstr(10);
                let val = dat.g4s();
                self.check_vars.push(HuntVar { var_id, condition, val });
            }
            250 => self.name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized hunt config code: {code}")),
        }
        Ok(())
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}
