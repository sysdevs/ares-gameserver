//! Inv ops and the player inventory helpers they call.
//! Wealth/session logging is dropped.

use super::super::runner::OpResult;
use super::super::state::{InvObjRef, ScriptState};
use ares_shared::opcode::op;
use super::super::PointerType;
use super::{coord, duration, in_range, int_param, not_null, valid};
use crate::cache::config::inv::{self as inv_cfg, InvType};
use crate::engine::entity::{NO_RECEIVER, Obj, lifecycle};
use crate::engine::inventory::{ALWAYS_STACK, Inventory, Item, STACK_LIMIT};
use super::player::ap;
use crate::engine::world::{InvOwner, World};

type R<T> = Result<T, String>;

/// The inventory owner, or the method's "Invalid inventory type" error.
fn owner(w: &mut World, slot: i32, inv: i32, method: &str) -> R<InvOwner> {
    w.player_inventory(slot, inv).ok_or_else(|| format!("{method}: Invalid inventory type: {inv}"))
}

fn inv<'a>(w: &'a mut World, o: InvOwner) -> &'a mut Inventory {
    w.inv_mut(o).expect("inventory")
}

/// The protected-access rule shared by the inv write ops (and `POP_VARO`).
pub(crate) fn protect(s: &ScriptState, t: &InvType, which: usize, prefix: &str) -> R<()> {
    let ptr = if which == 0 { PointerType::PActivePlayer } else { PointerType::PActivePlayer2 };
    if !s.pointer_get(ptr) && t.protect && t.scope != inv_cfg::SCOPE_SHARED {
        return Err(format!("{prefix} requires protected access: {}", t.name.clone().unwrap_or_else(|| t.id.to_string())));
    }
    Ok(())
}

/// The `to` check of the move ops tests the *from* inv's scope (a quirk kept on purpose).
fn protect_to(s: &ScriptState, to: &InvType, from: &InvType) -> R<()> {
    let which = (s.int_operand() != 0) as usize;
    let ptr = if which == 0 { PointerType::PActivePlayer } else { PointerType::PActivePlayer2 };
    if !s.pointer_get(ptr) && to.protect && from.scope != inv_cfg::SCOPE_SHARED {
        return Err(format!("$inv requires protected access: {}", to.name.clone().unwrap_or_else(|| to.id.to_string())));
    }
    Ok(())
}

/// `it` without its vars when it lands in a shared inv.
fn drop_vars(mut it: Item, shared: bool) -> Item {
    if shared {
        it.vars = None;
    }
    it
}

fn obj_stack(v: i32) -> R<i32> {
    in_range(v, 1, STACK_LIMIT as i32, "ObjStack")
}

pub(crate) fn inv_get_slot(w: &mut World, slot: i32, inv_id: i32, i: i32) -> R<Option<Item>> {
    let o = owner(w, slot, inv_id, "invGetSlot")?;
    let c = inv(w, o);
    if !c.valid_slot(i) {
        return Err(format!("invGetSlot: Invalid slot: {i}"));
    }
    Ok(c.get(i as usize))
}

pub(crate) fn inv_add(w: &mut World, slot: i32, inv_id: i32, obj: i32, count: i64) -> R<i64> {
    let o = owner(w, slot, inv_id, "invAdd")?;
    let (c, configs) = w.inv_with_configs(o).expect("inventory");
    Ok(c.add(&configs.objs, obj, count, -1))
}

/// `add_item` into a player inv: the number added.
fn inv_add_item(w: &mut World, slot: i32, inv_id: i32, item: Item) -> R<i64> {
    let o = owner(w, slot, inv_id, "invAdd")?;
    let (c, configs) = w.inv_with_configs(o).expect("inventory");
    Ok(c.add_item(&configs.objs, item, -1))
}

fn inv_set(w: &mut World, slot: i32, inv_id: i32, obj: i32, count: i64, i: i32) -> R<()> {
    let o = owner(w, slot, inv_id, "invSet")?;
    let c = inv(w, o);
    if !c.valid_slot(i) {
        return Err(format!("invSet: Invalid slot: {i}"));
    }
    c.set(i as usize, Some(Item::new(obj, count)));
    Ok(())
}

pub(crate) fn inv_del(w: &mut World, slot: i32, inv_id: i32, obj: i32, count: i64, begin: i32) -> R<i64> {
    let o = owner(w, slot, inv_id, "invDel")?;
    let size = inv(w, o).capacity as i32;
    if begin < -1 || begin >= size {
        return Err(format!("invDel: Invalid beginSlot: {begin}"));
    }
    let (c, configs) = w.inv_with_configs(o).expect("inventory");
    Ok(c.remove(&configs.invs, obj, count, begin))
}

fn inv_del_slot(w: &mut World, slot: i32, inv_id: i32, i: i32) -> R<()> {
    let o = owner(w, slot, inv_id, "invDelSlot")?;
    let c = inv(w, o);
    if !c.valid_slot(i) {
        return Err(format!("invDelSlot: Invalid slot: {i}"));
    }
    c.delete(i as usize);
    Ok(())
}

fn inv_total(w: &mut World, slot: i32, inv_id: i32, obj: i32) -> R<i64> {
    let o = owner(w, slot, inv_id, "invTotal")?;
    Ok(inv(w, o).item_count(obj))
}

fn inv_free_space(w: &mut World, slot: i32, inv_id: i32) -> R<i64> {
    let o = owner(w, slot, inv_id, "invFreeSpace")?;
    Ok(inv(w, o).free_slot_count() as i64)
}

fn inv_size(w: &mut World, slot: i32, inv_id: i32) -> R<i64> {
    let o = owner(w, slot, inv_id, "invSize")?;
    Ok(inv(w, o).capacity as i64)
}

/// How many would not fit.
fn inv_item_space(w: &mut World, slot: i32, inv_id: i32, obj: i32, count: i64, size: i64) -> R<i64> {
    let o = owner(w, slot, inv_id, "invItemSpace")?;
    let t = w.configs.objs.get(obj as usize).expect("obj");
    let uncert = if t.certtemplate >= 0 && t.certlink >= 0 { t.certlink } else { obj };
    let stackable = t.stackable;
    let always = inv(w, o).stack_type == ALWAYS_STACK;
    if stackable || uncert != obj || always {
        let stock = w.configs.invs.get(inv_id as usize).and_then(|t| t.stockobj.as_ref()).is_some_and(|s| s.iter().any(|&x| x as i32 == obj));
        if inv_total(w, slot, inv_id, obj)? == 0 && inv_free_space(w, slot, inv_id)? == 0 && !stock {
            return Ok(count);
        }
        return Ok((count - (STACK_LIMIT - inv_total(w, slot, inv_id, obj)?)).max(0));
    }
    Ok((count - (inv_free_space(w, slot, inv_id)? - (inv_size(w, slot, inv_id)? - size))).max(0))
}

/// Drops an overflow at the player's feet for 200 ticks, one per obj
/// unless stackable.
fn drop_overflow(w: &mut World, slot: i32, obj: i32, overflow: i64) {
    if overflow <= 0 {
        return;
    }
    let p = w.pl(slot);
    let (level, x, z, hash) = (p.e.level, p.e.x, p.e.z, p.hash64);
    let stackable = w.configs.objs.get(obj as usize).is_some_and(|t| t.stackable);
    if !stackable || overflow == 1 {
        for _ in 0..overflow {
            let id = w.new_obj(Obj::new(level, x, z, lifecycle::DESPAWN, obj, 1));
            w.add_obj(id, hash, 200);
        }
    } else {
        let id = w.new_obj(Obj::new(level, x, z, lifecycle::DESPAWN, obj, overflow));
        w.add_obj(id, hash, 200);
    }
}

pub fn run(w: &mut World, s: &mut ScriptState, opcode: i32) -> Option<OpResult> {
    super::handled((|| {
        match opcode {
            op::INV_ALLSTOCK | op::INV_SIZE | op::INV_DEBUGNAME => {
                let t = valid(&w.configs.invs, s.pop_int(), "Inv")?;
                match opcode {
                    op::INV_ALLSTOCK => s.push_int(t.allstock as i32),
                    op::INV_SIZE => s.push_int(t.size),
                    _ => s.push_string(t.name.clone().unwrap_or("null".into())),
                }
            }
            op::INV_STOCKBASE => {
                let [i, obj] = s.pop_ints::<2>();
                let t = valid(&w.configs.invs, i, "Inv")?;
                let id = valid(&w.configs.objs, obj, "Obj")?.id as i32;
                let v = match (&t.stockobj, &t.stockcount) {
                    (Some(objs), Some(counts)) => objs.iter().position(|&o| o as i32 == id).and_then(|i| counts.get(i)).map_or(-1, |&c| c as i32),
                    _ => -1,
                };
                s.push_int(v);
            }
            op::INV_ADD => {
                let [i, obj, count] = s.pop_ints::<3>();
                let t = valid(&w.configs.invs, i, "Inv")?;
                let o = valid(&w.configs.objs, obj, "Obj")?;
                obj_stack(count)?;
                protect(s, t, s.which(), "$inv")?;
                if !t.dummyinv && o.dummyitem != 0 {
                    return Err(format!("dummyitem in non-dummyinv: {} -> {}", o.debug_name.clone().unwrap_or_else(|| o.id.to_string()), t.name.clone().unwrap_or_else(|| t.id.to_string())));
                }
                let slot = ap(w, s)?;
                let added = inv_add(w, slot, i, obj, count as i64)?;
                drop_overflow(w, slot, obj, count as i64 - added);
            }
            op::INV_CHANGESLOT => {
                let [i, find, replace, replace_count] = s.pop_ints::<4>();
                let t = valid(&w.configs.invs, i, "Inv")?;
                protect(s, t, s.which(), "$inv")?;
                let find = valid(&w.configs.objs, find, "Obj")?.id as i32;
                let replace = valid(&w.configs.objs, replace, "Obj")?.id as i32;
                let slot = ap(w, s)?;
                let o = w.player_inventory(slot, i).ok_or("inv is null")?;
                let c = inv(w, o);
                if let Some(at) = (0..c.capacity).find(|&k| c.has_at(k, find)) {
                    // id and count change in place; vars stay (D5)
                    let mut it = c.get(at).expect("found");
                    (it.id, it.count) = (replace, replace_count as i64);
                    c.set(at, Some(it));
                }
            }
            op::INV_CLEAR => {
                let i = s.pop_int();
                let t = valid(&w.configs.invs, i, "Inv")?;
                protect(s, t, s.which(), "$inv")?;
                let slot = ap(w, s)?;
                let o = owner(w, slot, i, "invClear")?;
                inv(w, o).remove_all();
            }
            op::INV_DEL => {
                let [i, obj, count] = s.pop_ints::<3>();
                let t = valid(&w.configs.invs, i, "Inv")?;
                valid(&w.configs.objs, obj, "Obj")?;
                obj_stack(count)?;
                protect(s, t, s.which(), "$inv")?;
                let slot = ap(w, s)?;
                inv_del(w, slot, i, obj, count as i64, -1)?;
            }
            op::INV_DELSLOT => {
                let [i, at] = s.pop_ints::<2>();
                let t = valid(&w.configs.invs, i, "Inv")?;
                protect(s, t, s.which(), "$inv")?;
                let slot = ap(w, s)?;
                if inv_get_slot(w, slot, i, at)?.is_some() {
                    inv_del_slot(w, slot, i, at)?;
                }
            }
            op::INV_DROPITEM | op::INV_DROPITEM_DELAYED => {
                let delayed = opcode == op::INV_DROPITEM_DELAYED;
                let (i, c, obj, count, dur, delay) = if delayed {
                    let [a, b, c, d, e, f] = s.pop_ints::<6>();
                    (a, b, c, d, e, f)
                } else {
                    let [a, b, c, d, e] = s.pop_ints::<5>();
                    (a, b, c, d, e, 0)
                };
                let t = valid(&w.configs.invs, i, "Inv")?;
                let (level, x, z) = coord(c)?;
                let obj = valid(&w.configs.objs, obj, "Obj")?.id as i32;
                obj_stack(count)?;
                duration(dur)?;
                protect(s, t, s.which(), "$inv")?;
                let slot = ap(w, s)?;
                let completed = inv_del(w, slot, i, obj, count as i64, -1)?;
                if completed == 0 {
                    return Ok(());
                }
                let hash = w.pl(slot).hash64;
                let floor = Obj::new(level, x, z, lifecycle::DESPAWN, obj, completed);
                if delayed {
                    w.queue_delayed_obj(floor, hash, dur, delay);
                } else {
                    let id = w.new_obj(floor);
                    w.add_obj(id, hash, dur);
                    s.set_active_obj(id);
                }
            }
            op::INV_DROPSLOT => {
                let [i, c, at, dur] = s.pop_ints::<4>();
                valid(&w.configs.invs, i, "Inv")?;
                duration(dur)?;
                let (level, x, z) = coord(c)?;
                let t = valid(&w.configs.invs, i, "Inv")?;
                protect(s, t, s.which(), "$inv")?;
                let slot = ap(w, s)?;
                let item = inv_get_slot(w, slot, i, at)?.ok_or("$slot is empty")?;
                let completed = inv_del(w, slot, i, item.id, item.count, at)?;
                if completed == 0 {
                    return Ok(());
                }
                let hash = w.pl(slot).hash64;
                let stackable = w.configs.objs.get(item.id as usize).is_some_and(|t| t.stackable);
                if !stackable || completed == 1 {
                    for _ in 0..completed {
                        let id = w.new_obj(Obj::new(level, x, z, lifecycle::DESPAWN, item.id, 1));
                        w.add_obj(id, hash, dur);
                        s.set_active_obj(id);
                    }
                } else {
                    let id = w.new_obj(Obj::new(level, x, z, lifecycle::DESPAWN, item.id, completed));
                    w.add_obj(id, hash, dur);
                    s.set_active_obj(id);
                }
            }
            op::INV_FREESPACE => {
                let i = valid(&w.configs.invs, s.pop_int(), "Inv")?.id as i32;
                let slot = ap(w, s)?;
                let v = inv_free_space(w, slot, i)?;
                s.push_int(v as i32);
            }
            op::INV_GETNUM | op::INV_GETOBJ => {
                let [i, at] = s.pop_ints::<2>();
                let i = valid(&w.configs.invs, i, "Inv")?.id as i32;
                let slot = ap(w, s)?;
                let item = inv_get_slot(w, slot, i, at)?;
                match opcode {
                    op::INV_GETNUM => s.push_int(item.map_or(0, |it| it.count) as i32),
                    _ => s.push_int(item.map_or(-1, |it| it.id)),
                }
            }
            op::INV_SLOTOBJ => {
                let [i, at] = s.pop_ints::<2>();
                let i = valid(&w.configs.invs, i, "Inv")?.id as i32;
                let slot = ap(w, s)?;
                let held = inv_get_slot(w, slot, i, at)?.is_some();
                if held {
                    s.set_active_invobj(InvObjRef { pid: slot, inv: i, slot: at as usize });
                }
                s.push_int(held as i32);
            }
            op::INV_ITEMSPACE | op::INV_ITEMSPACE2 => {
                let [i, obj, count, size] = s.pop_ints::<4>();
                if count == 0 {
                    s.push_int(0);
                    return Ok(());
                }
                let t = valid(&w.configs.invs, i, "Inv")?;
                let tsize = t.size;
                valid(&w.configs.objs, obj, "Obj")?;
                obj_stack(count)?;
                if opcode == op::INV_ITEMSPACE && (size < 0 || size > tsize) {
                    return Err(format!("$count is out of range: {count}"));
                }
                let slot = ap(w, s)?;
                let space = inv_item_space(w, slot, i, obj, count as i64, size as i64)?;
                if opcode == op::INV_ITEMSPACE {
                    s.push_int((space == 0) as i32);
                } else {
                    s.push_int(space as i32);
                }
            }
            op::INV_MOVEFROMSLOT | op::INV_MOVETOSLOT => {
                let to_slot = if opcode == op::INV_MOVETOSLOT { Some(s.pop_int()) } else { None };
                let [from, to, from_slot] = s.pop_ints::<3>();
                let ft = valid(&w.configs.invs, from, "Inv")?;
                let tt = valid(&w.configs.invs, to, "Inv")?;
                protect(s, ft, s.which(), "$inv")?;
                protect_to(s, tt, ft)?;
                let (from_shared, to_shared) = (ft.scope == inv_cfg::SCOPE_SHARED, tt.scope == inv_cfg::SCOPE_SHARED);
                let slot = ap(w, s)?;
                match to_slot {
                    Some(to_slot) => {
                        let fo = owner(w, slot, from, "invMoveToSlot")?;
                        if !inv(w, fo).valid_slot(from_slot) {
                            return Err(format!("invMoveToSlot: Invalid from slot: {from_slot}"));
                        }
                        let tob = owner(w, slot, to, "invMoveToSlot")?;
                        if !inv(w, tob).valid_slot(to_slot) {
                            return Err(format!("invMoveToSlot: Invalid to slot: {to_slot}"));
                        }
                        // whole items swap, vars included (D5); a shared inv drops them
                        let a = inv(w, fo).get(from_slot as usize).map(|it| drop_vars(it, to_shared));
                        let b = inv(w, tob).get(to_slot as usize).map(|it| drop_vars(it, from_shared));
                        inv(w, tob).set(to_slot as usize, a);
                        inv(w, fo).set(from_slot as usize, b);
                    }
                    None => {
                        let fo = owner(w, slot, from, "invMoveFromSlot")?;
                        owner(w, slot, to, "invMoveFromSlot")?;
                        if !inv(w, fo).valid_slot(from_slot) {
                            return Err(format!("invMoveFromSlot: Invalid from slot: {from_slot}"));
                        }
                        let it = inv_get_slot(w, slot, from, from_slot)?.ok_or_else(|| format!("invMoveFromSlot: Invalid from obj was null. This means the obj does not exist at this slot: {from_slot}"))?;
                        inv_del_slot(w, slot, from, from_slot)?;
                        let it = drop_vars(it, to_shared);
                        let (id, count) = (it.id, it.count);
                        let added = inv_add_item(w, slot, to, it)?;
                        drop_overflow(w, slot, id, count - added);
                    }
                }
            }
            op::BOTH_MOVEINV => {
                let [from, to] = s.pop_ints::<2>();
                let ft = valid(&w.configs.invs, from, "Inv")?;
                let tt = valid(&w.configs.invs, to, "Inv")?;
                let secondary = s.int_operand() == 1;
                let (fp, tp) = if secondary { (s.active_player[1], s.active_player[0]) } else { (s.active_player[0], s.active_player[1]) };
                let (Some(fp), Some(tp)) = (fp, tp) else { return Err("player is null".into()) };
                protect(s, ft, secondary as usize, "$from_inv")?;
                let ptr = if secondary { PointerType::PActivePlayer } else { PointerType::PActivePlayer2 };
                if !s.pointer_get(ptr) && tt.protect && ft.scope != inv_cfg::SCOPE_SHARED {
                    return Err(format!("$to_inv requires protected access: {}", tt.name.clone().unwrap_or_else(|| tt.id.to_string())));
                }
                let to_shared = tt.scope == inv_cfg::SCOPE_SHARED;
                let fo =w.player_inventory(fp, from).ok_or("inv is null")?;
                let to_owner = w.player_inventory(tp, to).ok_or("inv is null")?;
                let to_kind = inv(w, to_owner).kind;
                let cap = inv(w, fo).capacity;
                for k in 0..cap {
                    let Some(it) = inv(w, fo).get(k) else { continue };
                    inv(w, fo).delete(k);
                    let it = drop_vars(it, to_shared);
                    let (id, count) = (it.id, it.count);
                    let added = inv_add_item(w, tp, to_kind, it)?;
                    drop_overflow(w, tp, id, count - added);
                }
            }
            op::INV_MOVEITEM | op::INV_MOVEITEM_CERT | op::INV_MOVEITEM_UNCERT => {
                let [from, to, obj, count] = s.pop_ints::<4>();
                let ft = valid(&w.configs.invs, from, "Inv")?;
                let tt = valid(&w.configs.invs, to, "Inv")?;
                let o = valid(&w.configs.objs, obj, "Obj")?;
                let (oid, certtemplate, certlink) = (o.id as i32, o.certtemplate, o.certlink);
                // uncert of a non-cert obj leaves it as is, so it carries vars like a plain move (D5)
                let keeps_obj = opcode == op::INV_MOVEITEM || (opcode == op::INV_MOVEITEM_UNCERT && !(certtemplate >= 0 && certlink >= 0));
                // a shared inv takes no vars: the item arrives plain and merges normally
                let carry = keeps_obj && tt.scope != inv_cfg::SCOPE_SHARED;
                obj_stack(count)?;
                protect(s, ft, s.which(), "$inv")?;
                protect_to(s, tt, ft)?;
                let slot = ap(w, s)?;
                let fo = owner(w, slot, from, "invDel")?;
                if carry && inv(w, fo).items.iter().flatten().any(|it| it.id == oid && it.vars.is_some()) {
                    // source slot order; a vars item moves whole, or as a copy
                    // of its vars for part of its stack (D5, D7)
                    let mut left = count as i64;
                    for k in 0..inv(w, fo).capacity {
                        if left <= 0 {
                            break;
                        }
                        let Some(mut it) = inv(w, fo).get(k).filter(|it| it.id == oid && it.count > 0) else { continue };
                        it.count = inv_del(w, slot, from, oid, it.count.min(left), k as i32)?;
                        left -= it.count;
                        let n = it.count;
                        let added = inv_add_item(w, slot, to, it)?;
                        drop_overflow(w, slot, oid, n - added);
                    }
                    return Ok(());
                }
                let completed = inv_del(w, slot, from, oid, count as i64, -1)?;
                if completed == 0 {
                    return Ok(());
                }
                match opcode {
                    op::INV_MOVEITEM => {
                        let added = inv_add(w, slot, to, oid, completed)?;
                        drop_overflow(w, slot, oid, count as i64 - added);
                    }
                    op::INV_MOVEITEM_CERT => {
                        let fin = if certtemplate == -1 && certlink >= 0 { certlink } else { oid };
                        let overflow = count as i64 - inv_add(w, slot, to, fin, completed)?;
                        if overflow > 0 {
                            let p = w.pl(slot);
                            let (level, x, z, hash) = (p.e.level, p.e.x, p.e.z, p.hash64);
                            let id = w.new_obj(Obj::new(level, x, z, lifecycle::DESPAWN, fin, overflow));
                            w.add_obj(id, hash, 200);
                        }
                    }
                    _ => {
                        let fin = if certtemplate >= 0 && certlink >= 0 { certlink } else { oid };
                        inv_add(w, slot, to, fin, completed)?;
                    }
                }
            }
            op::INV_SETSLOT => {
                let [i, at, obj, count] = s.pop_ints::<4>();
                let t = valid(&w.configs.invs, i, "Inv")?;
                let o = valid(&w.configs.objs, obj, "Obj")?;
                obj_stack(count)?;
                protect(s, t, s.which(), "$inv")?;
                if !t.dummyinv && o.dummyitem != 0 {
                    return Err(format!("dummyitem in non-dummyinv: {} -> {}", o.debug_name.clone().unwrap_or_else(|| o.id.to_string()), t.name.clone().unwrap_or_else(|| t.id.to_string())));
                }
                let slot = ap(w, s)?;
                inv_set(w, slot, i, obj, count as i64, at)?;
            }
            op::INV_TOTAL => {
                let [i, obj] = s.pop_ints::<2>();
                let i = valid(&w.configs.invs, i, "Inv")?.id as i32;
                if obj == -1 {
                    s.push_int(0);
                    return Ok(());
                }
                let slot = ap(w, s)?;
                let v = inv_total(w, slot, i, obj)?;
                s.push_int(v as i32);
            }
            op::INV_TOTALCAT => {
                let [i, cat] = s.pop_ints::<2>();
                let i = valid(&w.configs.invs, i, "Inv")?.id as i32;
                let cat = valid(&w.configs.categories, cat, "Cat")?.id as i32;
                let slot = ap(w, s)?;
                let o = owner(w, slot, i, "invTotalCat")?;
                let total: i64 = inv(w, o).items_filtered().iter().filter(|it| w.configs.objs.get(it.id as usize).is_some_and(|t| t.category == cat)).map(|it| it.count).sum();
                s.push_int(total as i32);
            }
            op::INV_TRANSMIT | op::INVOTHER_TRANSMIT => {
                let (uid, i, com) = if opcode == op::INVOTHER_TRANSMIT {
                    let [u, i, c] = s.pop_ints::<3>();
                    not_null(u)?;
                    (Some(u), i, c)
                } else {
                    let [i, c] = s.pop_ints::<2>();
                    (None, i, c)
                };
                let i = valid(&w.configs.invs, i, "Inv")?.id as i32;
                not_null(com)?;
                let slot = ap(w, s)?;
                let source = uid.map_or(w.pl(slot).uid as i64, |u| u as i64);
                w.inv_listen_on_com(slot, i, com, source);
            }
            op::INV_STOPTRANSMIT => {
                let com = not_null(s.pop_int())?;
                let slot = ap(w, s)?;
                w.inv_stop_listen_on_com(slot, com);
            }
            op::BOTH_DROPSLOT => {
                let [i, c, at, dur] = s.pop_ints::<4>();
                let t = valid(&w.configs.invs, i, "Inv")?;
                duration(dur)?;
                let (level, x, z) = coord(c)?;
                let secondary = s.int_operand() == 1;
                let (fp, tp) = if secondary { (s.active_player[1], s.active_player[0]) } else { (s.active_player[0], s.active_player[1]) };
                let (Some(fp), Some(tp)) = (fp, tp) else { return Err("player is null".into()) };
                protect(s, t, secondary as usize, "inv")?;
                let item = inv_get_slot(w, fp, i, at)?.ok_or("$slot is empty")?;
                let completed = inv_del(w, fp, i, item.id, item.count, at)?;
                if completed == 0 {
                    return Ok(());
                }
                let tradeable = w.configs.objs.get(item.id as usize).is_some_and(|t| t.tradeable);
                let receiver = if tradeable { w.pl(tp).hash64 } else { w.pl(fp).hash64 };
                let id = w.new_obj(Obj::new(level, x, z, lifecycle::DESPAWN, item.id, completed));
                w.add_obj(id, receiver, dur);
            }
            op::INV_DROPALL => {
                let [i, c, dur] = s.pop_ints::<3>();
                let t = valid(&w.configs.invs, i, "Inv")?;
                duration(dur)?;
                let (level, x, z) = coord(c)?;
                protect(s, t, s.which(), "$inv")?;
                let slot = ap(w, s)?;
                let Some(o) = w.player_inventory(slot, i) else { return Ok(()) };
                let hash = w.pl(slot).hash64;
                let cap = inv(w, o).capacity;
                for k in 0..cap {
                    let Some(it) = inv(w, o).get(k) else { continue };
                    inv(w, o).delete(k);
                    let tradeable = w.configs.objs.get(it.id as usize).is_some_and(|t| t.tradeable);
                    let id = w.new_obj(Obj::new(level, x, z, lifecycle::DESPAWN, it.id, it.count));
                    w.add_obj(id, if tradeable { NO_RECEIVER } else { hash }, dur);
                }
            }
            op::INV_TOTALPARAM | op::INV_TOTALPARAM_STACK => {
                let [i, param] = s.pop_ints::<2>();
                let slot = ap(w, s)?;
                let o = owner(w, slot, i, "invTotalParam")?;
                let p = w.configs.params.get(param as usize).ok_or_else(|| format!("no ParamType {param}"))?;
                let (pid, default) = (p.id as i32, p.default_int);
                let c = w.inv(o).expect("inventory");
                let mut total: i64 = 0;
                for it in c.items.iter().take(c.capacity).flatten() {
                    if it.id < 0 || it.id as usize >= w.configs.objs.count() {
                        continue;
                    }
                    let v = int_param(Some(&w.configs.objs.get(it.id as usize).unwrap().params), pid, default) as i64;
                    total += if opcode == op::INV_TOTALPARAM_STACK { it.count * v } else { v };
                }
                s.push_int(total as i32);
            }
            _ => return Err(super::UNHANDLED.into()),
        }
        Ok(())
    })())
}
