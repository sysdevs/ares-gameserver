//! Serving cache archives
//! 1-4 to clients on their on-demand connection.
//!
//! Chunk order *between* clients depends on timing and is not part of the
//! replay parity. Per client the rules are deterministic and
//! kept exactly: priority 2 first, FIFO within a priority, a repeated request
//! is dropped unless it raises the priority (the old one is cancelled), a
//! request equal to the file being sent is dropped, more than 2048 pending
//! requests close the client, each pump visit sends at most 16 chunks or
//! 8000 bytes, chunks carry at most 500 bytes.
//! ponytail: no 8 ms budget; a pump visits every scheduled client once. Add a
//! deadline if one pump ever starves the socket tasks.

use crate::io::filestream::FileStream;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

const MAX_PENDING_PER_CLIENT: usize = 2048;
const MAX_BYTES_PER_CLIENT_SLICE: usize = 8000;
const MAX_CHUNKS_PER_CLIENT_SLICE: usize = 16;
const CHUNK: usize = 500;

/// One 4-byte client request (`OnDemand.onClientData`): `None` means the
/// client sent an archive above 3 or a priority above 2 and is closed.
pub fn decode_request(b: [u8; 4]) -> Option<(u8, u16, u8)> {
    let (archive, file, priority) = (b[0], u16::from_be_bytes([b[1], b[2]]), b[3]);
    (archive <= 3 && priority <= 2).then_some((archive, file, priority))
}

/// `encodeChunk`: a 6-byte header (`archive, file u16, length u16, part`, each
/// truncated like the `Uint8Array` stores) then the payload.
pub fn encode_chunk(archive: u8, file: u16, length: usize, part: usize, payload: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(6 + payload.len());
    p.extend_from_slice(&[archive, (file >> 8) as u8, file as u8, (length >> 8) as u8, length as u8, part as u8]);
    p.extend_from_slice(payload);
    p
}

#[derive(Clone)]
struct Request {
    archive: u8,
    file: u16,
    priority: u8,
    cancelled: bool,
}

struct Active {
    archive: u8,
    file: u16,
    data: Option<Vec<u8>>,
    pos: usize,
    part: usize,
}

#[derive(Default)]
struct ClientQueue {
    queues: [VecDeque<usize>; 3],
    requests: Vec<Request>,
    pending_by_key: HashMap<(u8, u16), usize>,
    pending_count: usize,
    active: Option<Active>,
    scheduled: bool,
}

pub enum Outcome {
    Queued,
    Ignored,
    /// The client must be closed (`close_client`).
    Close,
}

pub struct OnDemand<K> {
    clients: HashMap<K, ClientQueue>,
    round_robin: VecDeque<K>,
}

impl<K: Hash + Eq + Clone> Default for OnDemand<K> {
    fn default() -> Self {
        OnDemand { clients: HashMap::new(), round_robin: VecDeque::new() }
    }
}

impl<K: Hash + Eq + Clone> OnDemand<K> {
    /// `enqueue(msg)`.
    pub fn request(&mut self, client: &K, archive: u8, file: u16, priority: u8) -> Outcome {
        if archive > 3 || priority > 2 {
            self.client_closed(client);
            return Outcome::Close;
        }
        let c = self.clients.entry(client.clone()).or_default();
        let key = (archive, file);
        if c.active.as_ref().is_some_and(|a| (a.archive, a.file) == key) {
            return Outcome::Ignored;
        }
        if let Some(&i) = c.pending_by_key.get(&key) {
            if c.requests[i].priority >= priority {
                return Outcome::Ignored;
            }
            c.requests[i].cancelled = true;
            c.pending_count -= 1;
        }
        if c.pending_count >= MAX_PENDING_PER_CLIENT {
            self.client_closed(client);
            return Outcome::Close;
        }
        c.requests.push(Request { archive, file, priority, cancelled: false });
        let i = c.requests.len() - 1;
        c.queues[priority as usize].push_back(i);
        c.pending_by_key.insert(key, i);
        c.pending_count += 1;
        if !c.scheduled {
            c.scheduled = true;
            self.round_robin.push_back(client.clone());
        }
        Outcome::Queued
    }

    /// `client_closed` / `deleteClient`.
    pub fn client_closed(&mut self, client: &K) {
        self.clients.remove(client);
    }

    pub fn has_work(&self) -> bool {
        !self.round_robin.is_empty()
    }

    /// One `pump()`: each scheduled client gets one slice; chunks are pushed
    /// to `out` in send order. `cache` is opened read-only on `data/pack`.
    pub fn pump(&mut self, cache: &mut FileStream, out: &mut Vec<(K, Vec<u8>)>) {
        for _ in 0..self.round_robin.len() {
            let Some(id) = self.round_robin.pop_front() else { break };
            let Some(c) = self.clients.get_mut(&id) else { continue };
            c.scheduled = false;
            serve_client(c, cache, &id, out);
            if c.active.is_some() || c.pending_count > 0 {
                c.scheduled = true;
                self.round_robin.push_back(id);
            } else {
                self.clients.remove(&id);
            }
        }
    }
}

fn next_request(c: &mut ClientQueue) -> Option<Request> {
    for priority in (0..3).rev() {
        while let Some(i) = c.queues[priority].pop_front() {
            let r = c.requests[i].clone();
            if r.cancelled {
                continue;
            }
            c.pending_by_key.remove(&(r.archive, r.file));
            c.pending_count -= 1;
            return Some(r);
        }
    }
    None
}

fn serve_client<K: Clone>(c: &mut ClientQueue, cache: &mut FileStream, id: &K, out: &mut Vec<(K, Vec<u8>)>) {
    let (mut bytes, mut chunks) = (0, 0);
    while bytes < MAX_BYTES_PER_CLIENT_SLICE && chunks < MAX_CHUNKS_PER_CLIENT_SLICE {
        if c.active.is_none() {
            let Some(r) = next_request(c) else { return };
            let data = cache.read(r.archive as usize + 1, r.file as usize, false);
            c.active = Some(Active { archive: r.archive, file: r.file, data, pos: 0, part: 0 });
        }
        let a = c.active.as_mut().unwrap();
        let packet = match &a.data {
            None => encode_chunk(a.archive, a.file, 0, 0, &[]),
            Some(data) => {
                let n = CHUNK.min(data.len() - a.pos);
                let p = encode_chunk(a.archive, a.file, data.len(), a.part, &data[a.pos..a.pos + n]);
                a.pos += n;
                a.part += 1;
                p
            }
        };
        bytes += packet.len();
        chunks += 1;
        out.push((id.clone(), packet));
        if a.data.as_ref().is_none_or(|d| a.pos >= d.len()) {
            c.active = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cache with archive 1 files 0 (1200 bytes), 1 (10 bytes), 2 (empty slot);
/// stored files carry a 2-byte version trailer, which is served too.
    fn cache(name: &str) -> (std::path::PathBuf, FileStream) {
        let dir = std::env::temp_dir().join(format!("ares-ondemand-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut fs = FileStream::open(&dir, true).unwrap();
        fs.write(1, 0, &(0..1200).map(|i| i as u8).collect::<Vec<_>>(), 1);
        fs.write(1, 1, &[7; 10], 1);
        (dir, fs)
    }

    fn drain(od: &mut OnDemand<u8>, fs: &mut FileStream) -> Vec<(u8, Vec<u8>)> {
        let mut out = Vec::new();
        while od.has_work() {
            od.pump(fs, &mut out);
        }
        out
    }

    #[test]
    fn chunks_priorities_and_dedup() {
        let (dir, mut fs) = cache("order");
        let mut od = OnDemand::default();
        assert!(matches!(od.request(&1, 0, 0, 0), Outcome::Queued));
        assert!(matches!(od.request(&1, 0, 1, 0), Outcome::Queued));
        assert!(matches!(od.request(&1, 0, 0, 0), Outcome::Ignored)); // same priority
        assert!(matches!(od.request(&1, 0, 1, 2), Outcome::Queued)); // raised: old cancelled
        assert!(matches!(od.request(&1, 0, 2, 1), Outcome::Queued)); // missing file
        let out = drain(&mut od, &mut fs);
        let heads: Vec<[u8; 6]> = out.iter().map(|(_, p)| p[..6].try_into().unwrap()).collect();
        assert_eq!(
            heads,
            [[0, 0, 1, 0, 12, 0], [0, 0, 2, 0, 0, 0], [0, 0, 0, 4, 178, 0], [0, 0, 0, 4, 178, 1], [0, 0, 0, 4, 178, 2]],
            "priority 2, then 1, then 0; 1200 bytes + version trailer in 500/500/202"
        );
        assert_eq!(out[4].1.len(), 6 + 202);
        assert_eq!(&out[3].1[6..8], &[244, 245]); // bytes 500, 501
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn slice_limits_and_round_robin() {
        let (dir, mut fs) = cache("slices");
        let mut od = OnDemand::default();
        for f in 0..20 {
            od.request(&1, 0, 1, 0); // only the first is queued
            od.request(&2, 0, f % 2, 0);
        }
        let mut out = Vec::new();
        od.pump(&mut fs, &mut out);
        let who: Vec<u8> = out.iter().map(|(k, _)| *k).collect();
        // client 1: one 10-byte file; client 2: file 0 (3 chunks) + file 1
        assert_eq!(who, [1, 2, 2, 2, 2]);
        assert!(matches!(od.request(&3, 4, 0, 0), Outcome::Close));
        for f in 0..2049u16 {
            if let Outcome::Close = od.request(&4, 1, f, 0) {
                assert_eq!(f, 2048);
            }
        }
        assert!(decode_request([3, 1, 2, 2]) == Some((3, 258, 2)) && decode_request([4, 0, 0, 0]).is_none() && decode_request([0, 0, 0, 3]).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
