//! Inventories. Counts behave as JS doubles: `add` clamps a
//! stack to `STACK_LIMIT`, but nothing rejects a negative `add`/`remove`
//! count, so a stack can reach 0, go negative, or (removing a negative
//! amount) pass 2^31. Counts are therefore `i64` here.

use crate::cache::config::{ConfigSet, inv::InvType, obj::ObjType, var::VarObjType};
use std::collections::BTreeSet;

pub const STACK_LIMIT: i64 = 0x7fffffff;
pub const NORMAL_STACK: i32 = 0;
pub const ALWAYS_STACK: i32 = 1;
pub const NEVER_STACK: i32 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: i32,
    pub count: i64,
    /// Per-instance varo values, `configs.varos.count()` long; allocated by
    /// the first `POP_VARO` to the item (D3). Never sent to the client.
    pub vars: Option<Box<[i32]>>,
}

impl Item {
    pub fn new(id: i32, count: i64) -> Self {
        Item { id, count, vars: None }
    }

    /// A fresh `vars`: every varo at its type default (`varn_reset`, D2).
    pub fn default_vars(varos: &ConfigSet<VarObjType>) -> Box<[i32]> {
        (0..varos.count()).map(|i| crate::engine::entity::npc::varn_reset(varos.get(i).unwrap().kind).unwrap_or(-1)).collect()
    }
}

#[derive(Clone, Debug)]
pub struct Inventory {
    pub stack_type: i32,
    pub capacity: usize,
    /// The inv config id.
    pub kind: i32,
    pub items: Vec<Option<Item>>,
    pub update: bool,
    dirty: BTreeSet<usize>,
}

/// The first empty slot at or after `from`; a negative `from` counts from the end.
fn index_of_free(items: &[Option<Item>], from: i32) -> Option<usize> {
    let len = items.len() as i32;
    let start = if from < 0 { (len + from).max(0) } else { from };
    (start as usize..items.len()).find(|&i| items[i].is_none())
}

impl Inventory {
    pub fn new(kind: i32, capacity: usize, stack_type: i32) -> Self {
        Inventory { stack_type, capacity, kind, items: vec![None; capacity], update: false, dirty: BTreeSet::new() }
    }

    /// `Inventory.fromType(inv)`: sized from the config, prefilled with its stock.
    pub fn from_type(invs: &ConfigSet<InvType>, inv: i32) -> Self {
        assert!(inv != -1, "Invalid inventory type");
        let t = invs.get(inv as usize).expect("inv type");
        let mut c = Inventory::new(inv, t.size.max(0) as usize, if t.stackall { ALWAYS_STACK } else { NORMAL_STACK });
        if let (Some(obj), Some(count)) = (&t.stockobj, &t.stockcount) {
            for (i, &id) in obj.iter().enumerate() {
                c.set(i, Some(Item::new(id as i32, count.get(i).copied().unwrap_or(0) as i64)));
            }
        }
        c
    }

    pub fn has_at(&self, slot: usize, id: i32) -> bool {
        self.items.get(slot).and_then(Option::as_ref).is_some_and(|i| i.id == id)
    }

    pub fn next_free_slot(&self) -> Option<usize> {
        index_of_free(&self.items, 0)
    }

    pub fn free_slot_count(&self) -> usize {
        self.items.iter().filter(|i| i.is_none()).count()
    }

    pub fn items_filtered(&self) -> Vec<Item> {
        self.items.iter().flatten().cloned().collect()
    }

    pub fn item_count(&self, id: i32) -> i64 {
        let sum: i64 = self.items[..self.capacity].iter().flatten().filter(|i| i.id == id).map(|i| i.count).sum();
        sum.min(STACK_LIMIT)
    }

    pub fn item_index(&self, id: i32) -> Option<usize> {
        self.items.iter().position(|i| i.as_ref().is_some_and(|i| i.id == id))
    }

    /// The stack `add` merges into: like `item_index`, but an item with vars
    /// never takes part in a merge (D4).
    fn stack_index(&self, id: i32) -> Option<usize> {
        self.items.iter().position(|i| i.as_ref().is_some_and(|i| i.id == id && i.vars.is_none()))
    }

    /// `add` for a moved item, the number added. An item with vars is placed
    /// whole in its own free slot (D4), or not at all.
    pub fn add_item(&mut self, objs: &ConfigSet<ObjType>, item: Item, begin_slot: i32) -> i64 {
        if item.vars.is_none() {
            return self.add(objs, item.id, item.count, begin_slot);
        }
        let index = if begin_slot == -1 { self.next_free_slot() } else { index_of_free(&self.items, begin_slot) };
        let Some(index) = index else { return 0 };
        let count = item.count;
        self.set(index, Some(item));
        count
    }

    pub fn remove_all(&mut self) {
        for slot in 0..self.capacity {
            if self.items[slot].is_some() {
                self.items[slot] = None;
                self.mark_dirty(slot);
            }
        }
    }

    /// `add(id, count, beginSlot)`: the number added. Unstackable items fill
    /// free slots one each (at least one, even for `count <= 0`).
    pub fn add(&mut self, objs: &ConfigSet<ObjType>, id: i32, count: i64, begin_slot: i32) -> i64 {
        let stackable = objs.get(id as usize).is_some_and(|t| t.stackable);
        let stack = self.stack_type != NEVER_STACK && (stackable || self.stack_type == ALWAYS_STACK);
        let mut completed = 0i64;
        if !stack {
            for i in begin_slot.max(0) as usize..self.capacity {
                if self.items[i].is_some() {
                    continue;
                }
                self.set(i, Some(Item::new(id, 1)));
                completed += 1;
                if completed >= count {
                    break;
                }
            }
        } else {
            let index = self.stack_index(id).or_else(|| if begin_slot == -1 { self.next_free_slot() } else { index_of_free(&self.items, begin_slot) });
            let Some(index) = index else { return completed };
            let stack_count = self.items[index].as_ref().map_or(0, |i| i.count);
            let total = (stack_count + count).min(STACK_LIMIT);
            self.set(index, Some(Item::new(id, total)));
            completed = total - stack_count;
        }
        completed
    }

    /// `remove(id, count, beginSlot)`: the number removed, scanning from
    /// `beginSlot` to the end and then the slots before it. Stock objs of
    /// the inv type keep their slot at count 0.
    pub fn remove(&mut self, invs: &ConfigSet<InvType>, id: i32, count: i64, begin_slot: i32) -> i64 {
        let stock_obj = invs.get(self.kind as usize).and_then(|t| t.stockobj.as_ref()).is_some_and(|s| s.iter().any(|&o| o as i32 == id));
        let mut removed = 0i64;
        // one slot of the scan; `true` once enough is removed
        let take = |inv: &mut Inventory, i: usize, removed: &mut i64| -> bool {
            let Some(item) = inv.items[i].as_mut().filter(|it| it.id == id) else { return false };
            let n = item.count.min(count - *removed);
            *removed += n;
            item.count -= n;
            if item.count == 0 {
                if stock_obj {
                    // the slot stays for restocking, which must merge into it (D4)
                    item.vars = None;
                } else {
                    inv.items[i] = None;
                }
            }
            inv.mark_dirty(i);
            *removed >= count
        };
        let start = if begin_slot != -1 { begin_slot } else { 0 };
        // a negative start reads `undefined` until slot 0
        for i in start.max(0) as usize..self.capacity {
            if take(self, i, &mut removed) {
                break;
            }
        }
        // slots past the capacity read as `undefined`
        if begin_slot != -1 && removed < count {
            for i in 0..(begin_slot.max(0) as usize).min(self.capacity) {
                if take(self, i, &mut removed) {
                    break;
                }
            }
        }
        removed
    }

    pub fn delete(&mut self, slot: usize) {
        self.set(slot, None);
    }

    pub fn get(&self, slot: usize) -> Option<Item> {
        self.items.get(slot).cloned().flatten()
    }

    pub fn set(&mut self, slot: usize, item: Option<Item>) {
        self.items[slot] = item;
        self.mark_dirty(slot);
    }

    pub fn valid_slot(&self, slot: i32) -> bool {
        slot >= 0 && (slot as usize) < self.capacity
    }

    /// `getDirtySlots()`: ascending.
    pub fn dirty_slots(&self) -> Vec<usize> {
        self.dirty.iter().copied().collect()
    }

    pub fn reset_tracking(&mut self) {
        self.update = false;
        self.dirty.clear();
    }

    fn mark_dirty(&mut self, slot: usize) {
        self.dirty.insert(slot);
        self.update = true;
    }
}
