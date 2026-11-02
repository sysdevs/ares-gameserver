//! Seq config (server + client) and the part of the anim frames it needs:
//! each frame's delay byte.
//!
//! Anim frames load once per process, so a hot reload keeps the frame delays
//! from the first load.

use super::{ConfigSet, ConfigType};
use crate::io::filestream::FileStream;
use crate::io::packet::Packet;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Debug, Serialize)]
pub struct SeqType {
    pub id: usize,
    pub name: Option<String>,
    pub frame_count: i32,
    pub frames: Option<Vec<i32>>,
    pub iframes: Option<Vec<i32>>,
    pub delay: Option<Vec<i32>>,
    pub loops: i32,
    pub walkmerge: Option<Vec<i32>>,
    pub reachforward: bool,
    pub priority: i32,
    pub replaceheldleft: i32,
    pub replaceheldright: i32,
    pub maxloops: i32,
    pub preanim_move: i32,
    pub postanim_move: i32,
    pub duplicatebehaviour: i32,
    pub duration: i32,
    #[serde(skip)]
    anim_delays: Arc<Vec<i32>>,
}

/// `AnimFrame.load()` reduced to `instances[id].delay` (0 where absent):
/// every file of cache archive 2, later files overwriting earlier frame ids.
pub fn load_anim_delays(cache: &mut FileStream) -> Vec<i32> {
    let mut delays = vec![0; 65536];
    for file in 0..cache.file_count(2) {
        if let Some(src) = cache.read(2, file, true) {
            unpack_delays(&src, &mut delays);
        }
    }
    delays
}

/// `AnimFrame.unpack`: the frame ids come from the head block, the delays
/// from the delay block; the transform blocks affect neither.
fn unpack_delays(src: &[u8], delays: &mut [i32]) {
    if src.len() < 8 {
        return;
    }
    let mut meta = Packet::new(src.to_vec());
    meta.pos = src.len() - 8;
    let head_len = meta.g2() as usize;
    let del_pos = head_len + 2 + meta.g2() as usize + meta.g2() as usize;
    let mut head = Packet::new(src.to_vec());
    let mut del = Packet::new(src.to_vec());
    del.pos = del_pos;
    let total = head.g2();
    for _ in 0..total {
        let id = head.g2() as usize;
        delays[id] = del.g1();
        head.g1(); // group count
    }
}

impl SeqType {
    /// `SeqType.load(dir)` with the frame delays from [`load_anim_delays`].
    pub fn load(dir: impl AsRef<Path>, anim_delays: Arc<Vec<i32>>) -> Result<ConfigSet<SeqType>, String> {
        ConfigSet::load_with_client_init(dir, "seq.dat", |c: &mut SeqType| c.anim_delays = anim_delays.clone())
    }
}

impl ConfigType for SeqType {
    fn new(id: usize) -> Self {
        SeqType {
            id,
            name: None,
            frame_count: 0,
            frames: None,
            iframes: None,
            delay: None,
            loops: -1,
            walkmerge: None,
            reachforward: false,
            priority: 5,
            replaceheldleft: -1,
            replaceheldright: -1,
            maxloops: 99,
            preanim_move: -1,
            postanim_move: -1,
            duplicatebehaviour: 0,
            duration: 0,
            anim_delays: Arc::default(),
        }
    }

    fn decode(&mut self, code: i32, dat: &mut Packet) -> Result<(), String> {
        match code {
            1 => {
                self.frame_count = dat.g1();
                let n = self.frame_count as usize;
                let (mut frames, mut iframes, mut delay) = (vec![0; n], vec![0; n], vec![0; n]);
                for i in 0..n {
                    frames[i] = dat.g2();
                    iframes[i] = dat.g2();
                    if iframes[i] == 65535 {
                        iframes[i] = -1;
                    }
                    delay[i] = dat.g2();
                    if delay[i] == 0 {
                        delay[i] = self.anim_delays.get(frames[i] as usize).copied().unwrap_or(0);
                    }
                    if delay[i] == 0 {
                        delay[i] = 1;
                    }
                    self.duration += delay[i];
                }
                (self.frames, self.iframes, self.delay) = (Some(frames), Some(iframes), Some(delay));
            }
            2 => self.loops = dat.g2(),
            3 => {
                let count = dat.g1() as usize;
                let mut w: Vec<i32> = (0..count).map(|_| dat.g1()).collect();
                w.push(9999999);
                self.walkmerge = Some(w);
            }
            4 => self.reachforward = true,
            5 => self.priority = dat.g1(),
            6 => self.replaceheldleft = dat.g2(),
            7 => self.replaceheldright = dat.g2(),
            8 => self.maxloops = dat.g1(),
            9 => self.preanim_move = dat.g1(),
            10 => self.postanim_move = dat.g1(),
            11 => self.duplicatebehaviour = dat.g1(),
            250 => self.name = Some(dat.gjstr(10)),
            _ => return Err(format!("Unrecognized seq config code: {code}")),
        }
        Ok(())
    }

    fn post_decode(&mut self) {
        if self.frame_count == 0 {
            self.frame_count = 1;
            self.frames = Some(vec![-1]);
            self.iframes = Some(vec![-1]);
            self.delay = Some(vec![-1]);
        }
        let merged = if self.walkmerge.is_none() { 0 } else { 2 };
        if self.preanim_move == -1 {
            self.preanim_move = merged;
        }
        if self.postanim_move == -1 {
            self.postanim_move = merged;
        }
    }

    fn debug_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}
