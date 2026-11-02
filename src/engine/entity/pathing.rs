//! The movement, interaction and info-mask state shared by players and
//! NPCs. Behaviour that touches the map lives on `World`; what only needs
//! this struct is here.

use crate::engine::coord;
use crate::script::state::EntityRef;

/// `MoveSpeed`.
pub mod move_speed {
    pub const STATIONARY: i32 = 0;
    pub const CRAWL: i32 = 1;
    pub const WALK: i32 = 2;
    pub const RUN: i32 = 3;
    pub const INSTANT: i32 = 4;
}

/// `MoveStrategy`.
pub mod move_strategy {
    pub const SMART: i32 = 0;
    pub const NAIVE: i32 = 1;
    pub const FLY: i32 = 2;
}

/// `MoveRestrict`.
pub mod move_restrict {
    pub const NORMAL: i32 = 0;
    pub const BLOCKED: i32 = 1;
    pub const BLOCKED_NORMAL: i32 = 2;
    pub const INDOORS: i32 = 3;
    pub const OUTDOORS: i32 = 4;
    pub const NOMOVE: i32 = 5;
    pub const PASSTHRU: i32 = 6;
}

#[derive(Clone, Debug)]
pub struct Pathing {
    pub block_walk: i32,
    pub move_strategy: i32,
    coordmask: u32,
    entitymask: u32,

    pub move_speed: i32,
    pub walk_dir: i32,
    pub run_dir: i32,
    pub waypoint_index: i32,
    pub waypoints: [i32; 25],
    pub last_tick_x: i32,
    pub last_tick_z: i32,
    pub last_level: i32,
    pub tele: bool,
    pub jump: bool,
    pub last_step_x: i32,
    pub last_step_z: i32,
    pub follow_x: i32,
    pub follow_z: i32,
    pub repathed: bool,
    pub steps_taken: i32,
    pub last_int: i32,
    pub last_crawl: bool,
    pub last_movement: i32,
    pub walktrigger: i32,
    pub walktrigger_arg: i32,
    pub delayed: bool,
    pub delayed_until: i32,
    pub interacted: bool,
    pub target: Option<EntityRef>,
    pub target_op: i32,
    pub target_subject_type: i32,
    pub target_subject_com: i32,
    pub ap_range: i32,
    pub ap_range_called: bool,
    /// Fine coords of a loc/obj target, consumed by `reorient`.
    pub target_x: i32,
    pub target_z: i32,
    pub face_angle_x: i32,
    pub face_angle_z: i32,

    pub masks: u32,
    pub exact_start_x: i32,
    pub exact_start_z: i32,
    pub exact_end_x: i32,
    pub exact_end_z: i32,
    pub exact_move_start: i32,
    pub exact_move_end: i32,
    pub exact_move_facing: i32,
    pub face_square_x: i32,
    pub face_square_z: i32,
    pub face_entity: i32,
    pub hitmark_slot: i32,
    pub hitmark_damage: i32,
    pub hitmark_type: i32,
    pub hitmark2_damage: i32,
    pub hitmark2_type: i32,
    pub anim_id: i32,
    pub anim_delay: i32,
    pub say_message: Option<String>,
    pub spotanim_id: i32,
    pub spotanim_height: i32,
    pub spotanim_time: i32,
}

impl Pathing {
    pub fn new(x: i32, z: i32, block_walk: i32, move_strategy: i32, coordmask: u32, entitymask: u32) -> Self {
        Pathing {
            block_walk,
            move_strategy,
            coordmask,
            entitymask,
            move_speed: move_speed::INSTANT,
            walk_dir: -1,
            run_dir: -1,
            waypoint_index: -1,
            waypoints: [0; 25],
            last_tick_x: -1,
            last_tick_z: -1,
            last_level: -1,
            tele: false,
            jump: false,
            last_step_x: x - 1,
            last_step_z: z,
            follow_x: -1,
            follow_z: -1,
            repathed: false,
            steps_taken: 0,
            last_int: -1,
            last_crawl: false,
            last_movement: 0,
            walktrigger: -1,
            walktrigger_arg: 0,
            delayed: false,
            delayed_until: -1,
            interacted: false,
            target: None,
            target_op: -1,
            target_subject_type: -1,
            target_subject_com: -1,
            ap_range: 10,
            ap_range_called: false,
            target_x: -1,
            target_z: -1,
            face_angle_x: -1,
            face_angle_z: -1,
            masks: 0,
            exact_start_x: -1,
            exact_start_z: -1,
            exact_end_x: -1,
            exact_end_z: -1,
            exact_move_start: -1,
            exact_move_end: -1,
            exact_move_facing: -1,
            face_square_x: -1,
            face_square_z: -1,
            face_entity: -1,
            hitmark_slot: 0,
            hitmark_damage: -1,
            hitmark_type: -1,
            hitmark2_damage: -1,
            hitmark2_type: -1,
            anim_id: -1,
            anim_delay: -1,
            say_message: None,
            spotanim_id: -1,
            spotanim_height: -1,
            spotanim_time: -1,
        }
    }

    pub fn queue_waypoint(&mut self, x: i32, z: i32) {
        self.waypoints[0] = coord::pack(0, x, z);
        self.waypoint_index = 0;
    }

    /// `queueWaypoints`: reversed, at most 25.
    pub fn queue_waypoints(&mut self, waypoints: &[u32]) {
        let mut index = -1;
        for (out, &w) in waypoints.iter().rev().take(self.waypoints.len()).enumerate() {
            self.waypoints[out] = w as i32;
            index += 1;
        }
        self.waypoint_index = index;
    }

    pub fn clear_waypoints(&mut self) {
        self.waypoint_index = -1;
    }

    pub fn has_waypoints(&self) -> bool {
        self.waypoint_index != -1
    }

    pub fn is_last_waypoint(&self) -> bool {
        self.waypoint_index <= 0
    }

    /// `focus(fineX, fineZ, client)`.
    pub fn focus(&mut self, fine_x: i32, fine_z: i32, client: bool) {
        self.face_angle_x = fine_x;
        self.face_angle_z = fine_z;
        if client {
            self.face_square_x = fine_x;
            self.face_square_z = fine_z;
            self.masks |= self.coordmask;
        }
    }

    /// `setFaceEntity()` given the target's face id (-1 for none).
    pub fn set_face_entity(&mut self, face: i32) {
        if self.face_entity != face {
            self.face_entity = face;
            self.masks |= self.entitymask;
        }
    }

    /// `resetPathingEntity()` up to (not including) `setFaceEntity`.
    pub fn reset(&mut self, default_speed: i32, x: i32, z: i32, level: i32) {
        self.move_speed = default_speed;
        self.walk_dir = -1;
        self.run_dir = -1;
        self.jump = false;
        self.tele = false;
        self.last_tick_x = x;
        self.last_tick_z = z;
        self.last_level = level;
        self.steps_taken = 0;
        self.interacted = false;
        self.ap_range_called = false;
        self.masks = 0;
        self.exact_start_x = -1;
        self.exact_start_z = -1;
        self.exact_end_x = -1;
        self.exact_end_z = -1;
        self.exact_move_start = -1;
        self.exact_move_end = -1;
        self.exact_move_facing = -1;
        self.anim_id = -1;
        self.anim_delay = -1;
        self.say_message = None;
        self.hitmark_damage = -1;
        self.hitmark_type = -1;
        self.hitmark2_damage = -1;
        self.hitmark2_type = -1;
        self.hitmark_slot = 0;
        self.spotanim_id = -1;
        self.spotanim_height = -1;
        self.spotanim_time = -1;
        self.face_square_x = -1;
        self.face_square_z = -1;
    }

    /// `PathingEntity.clearInteraction()`.
    pub fn clear_interaction(&mut self) {
        self.target = None;
        self.target_op = -1;
        self.target_subject_type = -1;
        self.target_subject_com = -1;
        self.ap_range = 10;
        self.ap_range_called = false;
    }
}
