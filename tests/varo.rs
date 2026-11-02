//! varo: per-instance obj vars on inventory items, read and written through
//! `active_invobj` (set by `inv_slotobj` and the opheld/inv_button triggers).
//! Runs the `07-varo` fixture procs against a hand-built World.

use ares_gameserver::engine::{configs::Configs, entity::player::Player, inventory::Item, world::World};
use ares_gameserver::script::{PointerType, provider::ScriptProvider, runner, state::ScriptState};
use std::path::PathBuf;

fn world() -> World {
    let pack = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/07-varo/data/pack");
    let mut w = World::new(true, Configs::load(&pack, true).unwrap());
    w.scripts = ScriptProvider::load(&pack).unwrap();
    for pid in [1, 2] {
        w.players[pid as usize] = Some(Box::new(Player::new(format!("p{pid}"), pid, pid as i64, &w.configs.varps, false)));
    }
    w
}

fn id(w: &World, kind: &str, name: &str) -> i32 {
    match kind {
        "inv" => w.configs.invs.get_id(name),
        "obj" => w.configs.objs.get_id(name),
        _ => w.configs.varos.get_id(name),
    }
}

/// Puts `item` in `slot` of player `pid`'s `inv`.
fn put(w: &mut World, pid: i32, slot: usize, item: Item) {
    let o = w.player_inventory(pid, id(w, "inv", "inv")).unwrap();
    w.inv_mut(o).unwrap().set(slot, Some(item));
}

fn slot(w: &mut World, pid: i32, inv: &str, slot: usize) -> Option<Item> {
    let o = w.player_inventory(pid, id(w, "inv", inv)).unwrap();
    w.inv_mut(o).unwrap().get(slot)
}

fn coins(w: &World, n: i64) -> Item {
    Item::new(id(w, "obj", "test_coins"), n)
}

/// Runs proc `name` as player 1 and returns its int results.
fn ints(w: &mut World, name: &str) -> Vec<i32> {
    let mut s = state(w, name);
    run(w, &mut s);
    assert_eq!(w.last_script_error, None);
    s.int_stack.to_vec()
}

fn ring(w: &World) -> Item {
    Item::new(id(w, "obj", "test_ring"), 1)
}

fn state(w: &World, name: &str) -> ScriptState {
    let script = w.scripts.get_by_name(&format!("[proc,{name}]")).unwrap_or_else(|| panic!("no proc {name}"));
    let mut s = ScriptState::new(script, &[]);
    s.set_active_player(1);
    s.pointer_add(PointerType::PActivePlayer);
    s
}

fn run(w: &mut World, s: &mut ScriptState) {
    w.last_script_error = None;
    runner::execute(w, s);
}

#[test]
fn varo_default_reads() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r);
    let mut s = state(&w, "varo_default_reads");
    run(&mut w, &mut s);
    assert_eq!(w.last_script_error, None);
    assert_eq!(s.int_stack, [0, -1]);
}

#[test]
fn varo_write_read() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r.clone());
    put(&mut w, 1, 1, r);
    let mut s = state(&w, "varo_write_read");
    run(&mut w, &mut s);
    assert_eq!(w.last_script_error, None);
    assert_eq!(s.int_stack, [5, 0]);
    let o = w.player_inventory(1, id(&w, "inv", "inv")).unwrap();
    let inv = w.inv_mut(o).unwrap();
    assert_eq!(inv.get(0).unwrap().vars.as_deref(), Some(&[5, -1][..]), "allocated with the type defaults");
    assert_eq!(inv.get(1).unwrap().vars, None);
}

#[test]
fn varo_null_pointer() {
    use ares_shared::opcode::op;
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r);
    let mut f = (*w.scripts.get_by_name("[proc,varo_slotobj_empty]").unwrap()).clone();
    (f.opcodes, f.int_operands, f.string_operands) = (vec![op::PUSH_VARO, op::RETURN], vec![0, 0], vec![None, None]);
    let mut s = ScriptState::new(std::sync::Arc::new(f), &[]);
    s.set_active_player(1);
    run(&mut w, &mut s);
    let e = w.last_script_error.unwrap_or_default();
    assert!(e.contains("null active_invobj"), "{e}");
}

#[test]
fn varo_slotobj_empty() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r);
    let mut s = state(&w, "varo_slotobj_empty");
    run(&mut w, &mut s);
    assert_eq!(w.last_script_error, None);
    assert_eq!(s.int_stack, [0]);
    assert_eq!(s.active_invobj, [None, None]);
    assert!(!s.pointer_get(PointerType::ActiveInvObj));
}

/// Review Focus 3: the pointer names slot 0, not the ring that moved out of it.
#[test]
fn varo_pointer_follows_slot() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r);
    let mut s = state(&w, "varo_pointer_follows_slot");
    run(&mut w, &mut s);
    let e = w.last_script_error.unwrap_or_default();
    assert!(e.contains("active_invobj slot is empty"), "{e}");
}

#[test]
fn varo_dot_form() {
    let mut w = world();
    let charges = id(&w, "varo", "test_charges") as usize;
    let mut held = ring(&w);
    let mut vars = vec![0, -1];
    vars[charges] = 7;
    held.vars = Some(vars.into_boxed_slice());
    let r = ring(&w);
    put(&mut w, 1, 0, r);
    put(&mut w, 2, 0, held);
    let mut s = state(&w, "varo_dot_form");
    s.active_player[1] = Some(2);
    s.pointer_add(PointerType::ActivePlayer2);
    run(&mut w, &mut s);
    assert_eq!(w.last_script_error, None);
    assert_eq!(s.int_stack, [7]);
    assert_eq!(s.active_invobj[0], None);
}

#[test]
fn run_player_trigger_sets_active_invobj() {
    let mut w = world();
    let charges = id(&w, "varo", "test_charges") as usize;
    let mut held = ring(&w);
    let mut vars = vec![0, -1];
    vars[charges] = 9;
    held.vars = Some(vars.into_boxed_slice());
    let r = ring(&w);
    put(&mut w, 1, 0, r);
    put(&mut w, 1, 3, held);
    let inv = id(&w, "inv", "inv");
    let p = w.player_mut(1).unwrap();
    (p.last_inv, p.last_slot) = (inv, 3);
    let script = w.scripts.get_by_name("[opheld1,test_ring]").unwrap();
    w.last_script_error = None;
    w.run_player_trigger(1, script, true).unwrap();
    assert_eq!(w.last_script_error, None);
    let varp = w.configs.varps.get_id("fixture_varp") as usize;
    assert_eq!(w.player(1).unwrap().vars[varp], 9);
}

#[test]
fn varo_movetoslot_swap() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r.clone());
    put(&mut w, 1, 5, r);
    assert_eq!(ints(&mut w, "varo_movetoslot_swap"), [7, 0]);
}

#[test]
fn varo_movefromslot_to_worn() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r);
    assert_eq!(ints(&mut w, "varo_movefromslot_to_worn"), [7]);
}

/// Review Focus 1: a vars stack and a plain stack of the same stackable obj
/// keep their own bank slots, and a later plain add merges only into the plain one.
#[test]
fn varo_bank_deposit_no_merge() {
    let mut w = world();
    let (a, b) = (coins(&w, 10), coins(&w, 5));
    put(&mut w, 1, 0, a);
    put(&mut w, 1, 1, b);
    assert_eq!(ints(&mut w, "varo_bank_deposit_no_merge"), [7, 0, 10, 8]);
}

#[test]
fn varo_both_moveinv() {
    let mut w = world();
    let (c, r) = (coins(&w, 3), ring(&w));
    put(&mut w, 1, 0, c);
    put(&mut w, 1, 1, r);
    let mut s = state(&w, "varo_both_moveinv");
    s.active_player[1] = Some(2);
    s.pointer_add(PointerType::ActivePlayer2);
    s.pointer_add(PointerType::PActivePlayer2);
    run(&mut w, &mut s);
    assert_eq!(w.last_script_error, None);
    assert_eq!(s.int_stack, [7]);
    assert_eq!(slot(&mut w, 2, "worn", 0).map(|i| i.count), Some(3));
}

/// Review Focus 2: two rings, only the second with vars; both reach the
/// stackall bank and the second keeps its own slot and vars.
#[test]
fn varo_moveitem_moves_instance() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r.clone());
    put(&mut w, 1, 1, r);
    assert_eq!(ints(&mut w, "varo_moveitem_moves_instance"), [0, 7, 2]);
    assert_eq!(slot(&mut w, 1, "inv", 0), None);
    assert_eq!(slot(&mut w, 1, "inv", 1), None);
}

#[test]
fn varo_moveitem_partial_stack_copies() {
    let mut w = world();
    let c = coins(&w, 10);
    put(&mut w, 1, 0, c);
    assert_eq!(ints(&mut w, "varo_moveitem_partial_stack_copies"), [7, 6, 7, 4]);
}

#[test]
fn varo_moveitem_cert_drops() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r);
    assert_eq!(ints(&mut w, "varo_moveitem_cert_drops"), [0]);
    let cert = id(&w, "obj", "cert_test_ring");
    assert_eq!(slot(&mut w, 1, "player_bank", 0).map(|i| i.id), Some(cert));
}

#[test]
fn varo_changeslot_keeps() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r);
    assert_eq!(ints(&mut w, "varo_changeslot_keeps"), [7]);
    assert_eq!(slot(&mut w, 1, "inv", 0).map(|i| (i.id, i.count)), Some((id(&w, "obj", "test_coins"), 1)));
}

#[test]
fn varo_drop_discards() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r);
    assert_eq!(ints(&mut w, "varo_drop_discards"), [0]);
}

/// Review Focus 5: a stock slot taken to count 0 drops its vars, so the
/// restocking add merges back into it.
#[test]
fn varo_stock_zero_clears_vars() {
    let mut w = world();
    assert_eq!(ints(&mut w, "varo_stock_zero_clears_vars"), [0]);
    assert_eq!(slot(&mut w, 1, "varo_stock", 0).map(|i| i.count), Some(3));
    assert_eq!(slot(&mut w, 1, "varo_stock", 1), None);
}

fn save(w: &World) -> Vec<u8> {
    w.players[1].as_ref().unwrap().save(&w.configs.varps, &w.configs.invs, &w.configs.varos)
}

fn load(w: &World, sav: &[u8]) -> Player {
    Player::load("p1", sav, &w.configs.varps, &w.configs.invs, &w.configs.varos, false).unwrap()
}

/// Replaces the first `from` in `sav` with `to` and rewrites the CRC trailer.
fn patch(sav: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let at = sav.windows(from.len()).position(|w| w == from).expect("pattern not in save");
    let mut b = [&sav[..at], to, &sav[at + from.len()..]].concat();
    let n = b.len() - 4;
    let crc = ares_gameserver::io::packet::Packet::getcrc(&b, 0, n);
    b[n..].copy_from_slice(&crc.to_be_bytes());
    b
}

/// The saved ring at inv slot 0: `p2(obj + 1)`, `p1(1)`.
fn ring_rec(w: &World) -> Vec<u8> {
    let o = id(w, "obj", "test_ring") + 1;
    vec![(o >> 8) as u8, o as u8, 1]
}

fn loaded_ring(w: &World, p: &Player) -> Item {
    p.invs[&id(w, "inv", "inv")].get(0).unwrap()
}

#[test]
fn varo_save_roundtrip() {
    let mut w = world();
    let charges = id(&w, "varo", "test_charges") as usize;
    let mut r = ring(&w);
    let mut vars = vec![0, -1];
    vars[charges] = 7;
    r.vars = Some(vars.into());
    put(&mut w, 1, 0, r.clone());
    let p = load(&w, &save(&w));
    assert_eq!(loaded_ring(&w, &p), r);
}

/// A v7 save (no per-slot var block) loads with no vars.
#[test]
fn varo_save_v7_loads() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r.clone());
    let v8 = save(&w);
    let mut v7 = patch(&v8, &[ring_rec(&w), vec![0]].concat(), &ring_rec(&w));
    v7 = patch(&v7, &[0x20, 0x04, 0, 8], &[0x20, 0x04, 0, 7]);
    let p = load(&w, &v7);
    assert_eq!(loaded_ring(&w, &p), r);
    assert!(p.invs.values().all(|inv| (0..inv.items.len()).all(|s| inv.get(s).is_none_or(|i| i.vars.is_none()))));
}

/// Review Focus 4: a var id past `varos.count()` (the pack lost a varo) is
/// dropped on load.
#[test]
fn varo_save_drops_unknown_id() {
    let mut w = world();
    let mut r = ring(&w);
    r.vars = Some(vec![7, -1].into());
    put(&mut w, 1, 0, r);
    let n = w.configs.varos.count() as u8;
    let rec = ring_rec(&w);
    let sav = patch(&save(&w), &[&rec[..], &[1, 0, 0, 0, 0, 0, 7]].concat(), &[&rec[..], &[1, 0, n, 0, 0, 0, 7]].concat());
    let p = load(&w, &sav);
    assert_eq!(loaded_ring(&w, &p).vars, None);
}

/// Content's bank and shop move with `inv_moveitem_uncert`: a non-cert obj
/// keeps its vars both ways.
#[test]
fn varo_bank_uncert_roundtrip() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r);
    assert_eq!(ints(&mut w, "varo_bank_uncert_roundtrip"), [7]);
}

/// A shared inv takes no vars: both rings arrive plain and merge (stackall).
#[test]
fn varo_shared_inv_drops_vars() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r.clone());
    put(&mut w, 1, 1, r);
    assert_eq!(ints(&mut w, "varo_shared_inv_drops_vars"), [0, 2, 2]);
    assert_eq!(slot(&mut w, 1, "varo_shared", 0).unwrap().vars, None);
}

/// A vars buffer made before a live reload added varos grows on write.
#[test]
fn varo_write_grows_short_buffer() {
    let mut w = world();
    let mut r = ring(&w);
    r.vars = Some(Box::new([]));
    put(&mut w, 1, 0, r);
    ints(&mut w, "varo_write_read");
    let charges = id(&w, "varo", "test_charges") as usize;
    let vars = slot(&mut w, 1, "inv", 0).unwrap().vars.unwrap();
    assert_eq!(vars.len(), w.configs.varos.count());
    assert_eq!(vars[charges], 5);
}

/// Writes follow the inv write ops' protected access.
#[test]
fn varo_write_needs_protected_access() {
    let mut w = world();
    let r = ring(&w);
    put(&mut w, 1, 0, r);
    let script = w.scripts.get_by_name("[proc,varo_write_read]").unwrap();
    let mut s = ScriptState::new(script, &[]);
    s.set_active_player(1);
    run(&mut w, &mut s);
    let e = w.last_script_error.clone().unwrap_or_default();
    assert!(e.contains("%varo requires protected access: inv"), "{e}");
    assert_eq!(slot(&mut w, 1, "inv", 0).unwrap().vars, None);
}
