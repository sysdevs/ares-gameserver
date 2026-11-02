//! Interface components from the client
//! `interface` jag (`data`) plus the custom `server/interface.dat` names.
//! Components are indexed by id and the table is sparse.

use crate::io::jagfile::Jagfile;
use crate::io::packet::Packet;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

pub const TYPE_LAYER: i32 = 0;
pub const TYPE_UNUSED: i32 = 1;
pub const TYPE_INVENTORY: i32 = 2;
pub const TYPE_RECT: i32 = 3;
pub const TYPE_TEXT: i32 = 4;
pub const TYPE_SPRITE: i32 = 5;
pub const TYPE_MODEL: i32 = 6;
pub const TYPE_INVENTORY_TEXT: i32 = 7;

pub const NO_BUTTON: i32 = 0;
pub const BUTTON: i32 = 1;
pub const TARGET_BUTTON: i32 = 2;
pub const CLOSE_BUTTON: i32 = 3;
pub const TOGGLE_BUTTON: i32 = 4;
pub const SELECT_BUTTON: i32 = 5;
pub const PAUSE_BUTTON: i32 = 6;

#[derive(Clone, Debug, Serialize)]
pub struct Component {
    pub id: i32,
    pub root_layer: i32,
    pub com_name: Option<String>,
    pub overlay: bool,
    pub com_type: i32,
    pub button_type: i32,
    pub client_code: i32,
    pub width: i32,
    pub height: i32,
    pub trans: i32,
    pub over_layer: i32,
    pub script_comparator: Option<Vec<u8>>,
    pub script_operand: Option<Vec<u16>>,
    pub scripts: Option<Vec<Vec<u16>>>,
    pub scroll: i32,
    pub hide: bool,
    pub draggable: bool,
    pub operable: bool,
    pub usable: bool,
    pub swappable: bool,
    pub margin_x: i32,
    pub margin_y: i32,
    pub inventory_slot_offset_x: Option<Vec<u16>>,
    pub inventory_slot_offset_y: Option<Vec<u16>>,
    pub inventory_slot_graphic: Option<Vec<Option<String>>>,
    pub iop: Option<Vec<Option<String>>>,
    pub fill: bool,
    pub center: bool,
    pub font: i32,
    pub shadowed: bool,
    pub text: Option<String>,
    pub active_text: Option<String>,
    pub colour: i32,
    pub active_colour: i32,
    pub over_colour: i32,
    pub active_over_colour: i32,
    pub graphic: Option<String>,
    pub active_graphic: Option<String>,
    pub model: i32,
    pub active_model: i32,
    pub anim: i32,
    pub active_anim: i32,
    pub zoom: i32,
    pub xan: i32,
    pub yan: i32,
    pub action_verb: Option<String>,
    pub action: Option<String>,
    pub action_target: i32,
    pub option: Option<String>,
    pub child_id: Option<Vec<u16>>,
    pub child_x: Option<Vec<u16>>,
    pub child_y: Option<Vec<u16>>,
}

impl Default for Component {
    fn default() -> Self {
        Component {
            id: -1,
            root_layer: -1,
            com_name: None,
            overlay: false,
            com_type: -1,
            button_type: -1,
            client_code: 0,
            width: 0,
            height: 0,
            trans: 0,
            over_layer: -1,
            script_comparator: None,
            script_operand: None,
            scripts: None,
            scroll: 0,
            hide: false,
            draggable: false,
            operable: false,
            usable: false,
            swappable: false,
            margin_x: 0,
            margin_y: 0,
            inventory_slot_offset_x: None,
            inventory_slot_offset_y: None,
            inventory_slot_graphic: None,
            iop: None,
            fill: false,
            center: false,
            font: 0,
            shadowed: false,
            text: None,
            active_text: None,
            colour: 0,
            active_colour: 0,
            over_colour: 0,
            active_over_colour: 0,
            graphic: None,
            active_graphic: None,
            model: -1,
            active_model: -1,
            anim: -1,
            active_anim: -1,
            zoom: 0,
            xan: 0,
            yan: 0,
            action_verb: None,
            action: None,
            action_target: -1,
            option: None,
            child_id: None,
            child_x: None,
            child_y: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct Components {
    pub components: Vec<Option<Component>>,
    names: HashMap<String, usize>,
}

/// `((v - 1) << 8) + g1` when the first byte is non-zero, else `zero`.
fn g1_ext(dat: &mut Packet, zero: i32) -> i32 {
    let v = dat.g1();
    if v == 0 { zero } else { ((v - 1) << 8) + dat.g1() }
}

impl Components {
    /// `Component.load(dir)`: empty when the jag or its `data` entry is missing.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, String> {
        let dir = dir.as_ref();
        let jag_path = dir.join("client/interface");
        if !jag_path.exists() {
            return Ok(Components::default());
        }
        let client = Jagfile::load(&jag_path).map_err(|e| format!("client/interface: {e}"))?;
        let Some(mut data) = client.read("data") else { return Ok(Components::default()) };
        let mut set = Components::decode(&mut data);
        let mut server = Packet::load(dir.join("server/interface.dat"), false).map_err(|e| format!("server/interface.dat: {e}"))?;
        set.decode_extra(&mut server)?;
        Ok(set)
    }

    pub fn decode(dat: &mut Packet) -> Self {
        let mut set = Components::default();
        dat.g2(); // count
        let mut root_layer = -1;
        while dat.available() > 0 {
            let mut id = dat.g2();
            if id == 65535 {
                root_layer = dat.g2();
                id = dat.g2();
            }
            let mut com = Component { id, root_layer, ..Default::default() };
            com.com_type = dat.g1();
            com.button_type = dat.g1();
            com.client_code = dat.g2();
            com.width = dat.g2();
            com.height = dat.g2();
            com.trans = dat.g1();
            com.over_layer = g1_ext(dat, -1);

            let comparators = dat.g1() as usize;
            if comparators > 0 {
                let (mut c, mut o) = (vec![0u8; comparators], vec![0u16; comparators]);
                for i in 0..comparators {
                    c[i] = dat.g1() as u8;
                    o[i] = dat.g2() as u16;
                }
                com.script_comparator = Some(c);
                com.script_operand = Some(o);
            }

            let scripts = dat.g1() as usize;
            if scripts > 0 {
                com.scripts = Some(
                    (0..scripts)
                        .map(|_| {
                            let n = dat.g2() as usize;
                            (0..n).map(|_| dat.g2() as u16).collect()
                        })
                        .collect(),
                );
            }

            match com.com_type {
                TYPE_LAYER => {
                    com.scroll = dat.g2();
                    com.hide = dat.gbool();
                    let n = dat.g2() as usize;
                    let (mut ids, mut xs, mut ys) = (vec![0u16; n], vec![0u16; n], vec![0u16; n]);
                    for i in 0..n {
                        ids[i] = dat.g2() as u16;
                        xs[i] = dat.g2s() as u16;
                        ys[i] = dat.g2s() as u16;
                    }
                    com.child_id = Some(ids);
                    com.child_x = Some(xs);
                    com.child_y = Some(ys);
                }
                TYPE_UNUSED => dat.pos += 10,
                TYPE_INVENTORY => {
                    com.draggable = dat.gbool();
                    com.operable = dat.gbool();
                    com.usable = dat.gbool();
                    com.swappable = dat.gbool();
                    com.margin_x = dat.g1();
                    com.margin_y = dat.g1();
                    let (mut xs, mut ys, mut gs) = (vec![0u16; 20], vec![0u16; 20], vec![None; 20]);
                    for i in 0..20 {
                        if dat.gbool() {
                            xs[i] = dat.g2s() as u16;
                            ys[i] = dat.g2s() as u16;
                            gs[i] = Some(dat.gjstr(10));
                        }
                    }
                    com.inventory_slot_offset_x = Some(xs);
                    com.inventory_slot_offset_y = Some(ys);
                    com.inventory_slot_graphic = Some(gs);
                    com.iop = Some((0..5).map(|_| Some(dat.gjstr(10))).collect());
                    com.action_verb = Some(dat.gjstr(10));
                    com.action = Some(dat.gjstr(10));
                    com.action_target = dat.g2();
                }
                TYPE_RECT => {
                    com.fill = dat.gbool();
                    com.colour = dat.g4s();
                    com.active_colour = dat.g4s();
                    com.over_colour = dat.g4s();
                    com.active_over_colour = dat.g4s();
                }
                TYPE_TEXT => {
                    com.center = dat.gbool();
                    com.font = dat.g1();
                    com.shadowed = dat.gbool();
                    com.text = Some(dat.gjstr(10));
                    com.active_text = Some(dat.gjstr(10));
                    com.colour = dat.g4s();
                    com.active_colour = dat.g4s();
                    com.over_colour = dat.g4s();
                    com.active_over_colour = dat.g4s();
                }
                TYPE_SPRITE => {
                    com.graphic = Some(dat.gjstr(10));
                    com.active_graphic = Some(dat.gjstr(10));
                }
                TYPE_MODEL => {
                    com.model = g1_ext(dat, 0);
                    com.active_model = g1_ext(dat, 0);
                    com.anim = g1_ext(dat, -1);
                    com.active_anim = g1_ext(dat, -1);
                    com.zoom = dat.g2();
                    com.xan = dat.g2();
                    com.yan = dat.g2();
                }
                TYPE_INVENTORY_TEXT => {
                    com.center = dat.gbool();
                    com.font = dat.g1();
                    com.shadowed = dat.gbool();
                    com.colour = dat.g4s();
                    com.margin_x = dat.g2s();
                    com.margin_y = dat.g2s();
                    com.operable = dat.gbool();
                    com.iop = Some((0..5).map(|_| Some(dat.gjstr(10))).collect());
                }
                _ => {}
            }

            match com.button_type {
                TARGET_BUTTON => {
                    com.action_verb = Some(dat.gjstr(10));
                    com.action = Some(dat.gjstr(10));
                    com.action_target = dat.g2();
                }
                BUTTON | TOGGLE_BUTTON | SELECT_BUTTON | PAUSE_BUTTON => com.option = Some(dat.gjstr(10)),
                _ => {}
            }

            let i = id as usize;
            if set.components.len() <= i {
                set.components.resize(i + 1, None);
            }
            set.components[i] = Some(com);
        }
        set
    }

    /// `Component.decodeExtra`: debug names and the overlay flag.
    fn decode_extra(&mut self, dat: &mut Packet) -> Result<(), String> {
        dat.g2(); // count
        while dat.available() > 0 {
            let id = dat.g2() as usize;
            let name = dat.gjstr(10);
            let overlay = dat.gbool();
            let com = self.components.get_mut(id).and_then(Option::as_mut).ok_or_else(|| format!("interface.dat names missing component {id}"))?;
            com.com_name = Some(name.clone());
            com.overlay = overlay;
            self.names.insert(name, id);
        }
        Ok(())
    }

    pub fn get(&self, id: usize) -> Option<&Component> {
        self.components.get(id).and_then(Option::as_ref)
    }

    pub fn get_id(&self, name: &str) -> i32 {
        self.names.get(name).map_or(-1, |&i| i as i32)
    }
}
