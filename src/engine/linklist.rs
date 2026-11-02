//! Intrusive linked lists (`LinkList`, `DoublyLinkList`, `HashTable`) as an
//! index arena.
//!
//! The lists are intrusive and their iteration order under mutation is
//! observable: `all()` caches the next link before yielding, so a node
//! appended while an earlier node is processed is still reached this pass
//! (the "queue speedup" in `Player.processQueue`), a node appended while the
//! last one is processed is not, and a cached next node that gets unlinked
//! is still yielded once (its links are cleared, which then ends the pass).
//! A `Vec` or a snapshot reproduces none of that, so every list here is a
//! sentinel node in a [`Links`] arena and iteration goes through [`Cursor`].
//!
//! One arena per link pair: an entity in two lists at once uses
//! `next/prev` for one and `next2/prev2` for the other, i.e. two arenas.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

#[derive(Clone, Copy, Debug, Default)]
struct Link {
    prev: Option<NodeId>,
    next: Option<NodeId>,
}

/// A list: its sentinel node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct List(NodeId);

/// Nodes carry a payload `T` (`None` for sentinels and freed nodes).
pub struct Links<T> {
    links: Vec<Link>,
    values: Vec<Option<T>>,
    free: Vec<NodeId>,
}

impl<T> Default for Links<T> {
    fn default() -> Self {
        Links { links: Vec::new(), values: Vec::new(), free: Vec::new() }
    }
}

/// An `all()` pass in progress: the saved `cursor` of the generator.
#[derive(Clone, Debug)]
pub struct Cursor {
    list: List,
    cursor: Option<NodeId>,
    started: bool,
    reverse: bool,
}

impl<T> Links<T> {
    fn alloc(&mut self, value: Option<T>) -> NodeId {
        match self.free.pop() {
            Some(id) => {
                self.links[id.0 as usize] = Link::default();
                self.values[id.0 as usize] = value;
                id
            }
            None => {
                self.links.push(Link::default());
                self.values.push(value);
                NodeId(self.links.len() as u32 - 1)
            }
        }
    }

    /// A new, unlinked node holding `value`.
    pub fn node(&mut self, value: T) -> NodeId {
        self.alloc(Some(value))
    }

    /// Drops a node's payload and recycles its id (unlinking it first).
    pub fn free(&mut self, id: NodeId) -> Option<T> {
        self.unlink(id);
        self.free.push(id);
        self.values[id.0 as usize].take()
    }

    /// A new empty list (`sentinel.next = sentinel.prev = sentinel`).
    pub fn list(&mut self) -> List {
        let s = self.alloc(None);
        self.links[s.0 as usize] = Link { prev: Some(s), next: Some(s) };
        List(s)
    }

    pub fn get(&self, id: NodeId) -> Option<&T> {
        self.values[id.0 as usize].as_ref()
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut T> {
        self.values[id.0 as usize].as_mut()
    }

    fn l(&self, id: NodeId) -> Link {
        self.links[id.0 as usize]
    }

    fn set_next(&mut self, id: NodeId, next: Option<NodeId>) {
        self.links[id.0 as usize].next = next;
    }

    fn set_prev(&mut self, id: NodeId, prev: Option<NodeId>) {
        self.links[id.0 as usize].prev = prev;
    }

    pub fn is_linked(&self, id: NodeId) -> bool {
        self.l(id).prev.is_some()
    }

    /// `Linkable.unlink()`.
    pub fn unlink(&mut self, id: NodeId) {
        let Link { prev, next } = self.l(id);
        if let Some(prev) = prev {
            self.set_next(prev, next);
            if let Some(next) = next {
                self.set_prev(next, Some(prev));
            }
            self.links[id.0 as usize] = Link::default();
        }
    }

    /// `addTail(node)`: moves the node from any list it is in.
    pub fn add_tail(&mut self, list: List, id: NodeId) {
        if self.l(id).prev.is_some() {
            self.unlink(id);
        }
        let s = list.0;
        let prev = self.l(s).prev;
        self.links[id.0 as usize] = Link { prev, next: Some(s) };
        if let Some(prev) = prev {
            self.set_next(prev, Some(id));
        }
        self.set_prev(s, Some(id));
    }

    /// `addHead(node)`.
    pub fn add_head(&mut self, list: List, id: NodeId) {
        if self.l(id).prev.is_some() {
            self.unlink(id);
        }
        let s = list.0;
        let next = self.l(s).next;
        self.links[id.0 as usize] = Link { prev: Some(s), next };
        self.set_next(s, Some(id));
        if let Some(next) = next {
            self.set_prev(next, Some(id));
        }
    }

    /// `removeHead()`.
    pub fn remove_head(&mut self, list: List) -> Option<NodeId> {
        let node = self.l(list.0).next?;
        if node == list.0 {
            return None;
        }
        self.unlink(node);
        Some(node)
    }

    /// `clear()`: unlinks every node (payloads stay allocated).
    pub fn clear(&mut self, list: List) {
        while let Some(node) = self.l(list.0).next {
            if node == list.0 {
                return;
            }
            self.unlink(node);
        }
    }

    pub fn is_empty(&self, list: List) -> bool {
        self.l(list.0).next == Some(list.0)
    }

    /// Starts an `all(reverse)` pass; advance it with [`Links::next`].
    pub fn iter(&self, list: List, reverse: bool) -> Cursor {
        Cursor { list, cursor: None, started: false, reverse }
    }

    /// The next node of an `all()` pass (`head()`/`next()` or
    /// `tail()`/`prev()` with the generator's saved cursor).
    pub fn next(&self, c: &mut Cursor) -> Option<NodeId> {
        let s = c.list.0;
        let step = |l: Link| if c.reverse { l.prev } else { l.next };
        let node = if !c.started {
            c.started = true;
            step(self.l(s))
        } else {
            c.cursor
        };
        match node {
            Some(n) if n == s => {
                c.cursor = None;
                None
            }
            Some(n) => {
                c.cursor = step(self.l(n));
                Some(n)
            }
            None => {
                c.cursor = None;
                None
            }
        }
    }

    /// The nodes of a list, front to back, without the cursor rules
    /// (for code that does not mutate while walking).
    pub fn collect(&self, list: List) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut c = self.iter(list, false);
        while let Some(n) = self.next(&mut c) {
            out.push(n);
        }
        out
    }
}

/// `HashTable`: `buckets` sentinel lists, a node's bucket is `key & (n-1)`.
pub struct HashTable {
    buckets: Vec<List>,
    keys: Vec<i128>,
}

impl HashTable {
    pub fn new<T>(links: &mut Links<T>, size: usize) -> Self {
        HashTable { buckets: (0..size).map(|_| links.list()).collect(), keys: Vec::new() }
    }

    fn bucket(&self, key: i128) -> List {
        self.buckets[(key & (self.buckets.len() as i128 - 1)) as usize]
    }

    /// `add(key, value)`: appended to its bucket (moved if already linked).
    pub fn add<T>(&mut self, links: &mut Links<T>, key: i128, id: NodeId) {
        links.add_tail(self.bucket(key), id);
        let i = id.0 as usize;
        if self.keys.len() <= i {
            self.keys.resize(i + 1, 0);
        }
        self.keys[i] = key;
    }

    pub fn find<T>(&self, links: &Links<T>, key: i128) -> Option<NodeId> {
        links.collect(self.bucket(key)).into_iter().find(|n| self.keys.get(n.0 as usize) == Some(&key))
    }

    /// `all()`: bucket by bucket; the next node is read before yielding, so
    /// the caller may unlink the yielded one (use [`HashTable::next`]).
    pub fn iter(&self) -> HashCursor {
        HashCursor { bucket: 0, node: None, started: false }
    }

    pub fn next<T>(&self, links: &Links<T>, c: &mut HashCursor) -> Option<NodeId> {
        loop {
            if c.bucket >= self.buckets.len() {
                return None;
            }
            let s = self.buckets[c.bucket].0;
            let node = if c.started { c.node } else { links.l(s).next };
            c.started = true;
            match node {
                Some(n) if n != s => {
                    c.node = links.l(n).next;
                    return Some(n);
                }
                _ => {
                    c.bucket += 1;
                    c.started = false;
                }
            }
        }
    }
}

pub struct HashCursor {
    bucket: usize,
    node: Option<NodeId>,
    started: bool,
}
