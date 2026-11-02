//! Song lengths in ms from cache archive 3, for
//! jingles and `midi_jingle`/track tick lengths.

use crate::io::filestream::FileStream;
use crate::util::numbers::js_round;

fn u16be(d: &[u8], o: usize) -> u32 {
    ((d[o] as u32) << 8) | d[o + 1] as u32
}

fn u32be(d: &[u8], o: usize) -> u64 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as u64
}

fn u32le(d: &[u8], o: usize) -> u64 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as u64
}

fn var_len(d: &[u8], mut o: usize, limit: usize) -> Option<(u32, usize)> {
    let mut value = 0u32;
    for _ in 0..4 {
        if o >= limit {
            return None;
        }
        let b = d[o];
        o += 1;
        value = (value << 7) | (b & 0x7f) as u32;
        if b & 0x80 == 0 {
            return Some((value, o));
        }
    }
    None
}

/// `unwrapRiffMidi`: `attack1.mid` has a RIFF/RMID wrapper.
fn unwrap_riff(d: &[u8]) -> Option<&[u8]> {
    if d.len() < 12 || &d[0..4] != b"RIFF" {
        return Some(d);
    }
    if &d[8..12] != b"RMID" {
        return None;
    }
    let mut o = 12;
    while o + 8 <= d.len() {
        let id = &d[o..o + 4];
        let size = u32le(d, o + 4) as usize;
        o += 8;
        if o + size > d.len() {
            return None;
        }
        if id == b"data" {
            return Some(&d[o..o + size]);
        }
        o += size + size % 2;
    }
    None
}

/// `parseMidiLength`: the song length in ms, `None` when malformed.
pub fn parse_midi_length(src: &[u8]) -> Option<f64> {
    let d = unwrap_riff(src)?;
    if d.len() < 14 || &d[0..4] != b"MThd" {
        return None;
    }
    let header_len = u32be(d, 4) as usize;
    let mut o = 8;
    if header_len < 6 || o + header_len > d.len() {
        return None;
    }
    let (format, tracks, division) = (u16be(d, o), u16be(d, o + 2), u16be(d, o + 4));
    o += header_len;
    if format > 2 || tracks == 0 {
        return None;
    }
    let mut max_tick = 0f64;
    let mut tempos: Vec<(f64, u32)> = Vec::new(); // (tick, tempo) in event order
    for _ in 0..tracks {
        if o + 8 > d.len() || &d[o..o + 4] != b"MTrk" {
            return None;
        }
        let track_end = o + 8 + u32be(d, o + 4) as usize;
        o += 8;
        if track_end > d.len() {
            return None;
        }
        let mut tick = 0f64;
        let mut running = 0u8;
        while o < track_end {
            let (delta, next) = var_len(d, o, track_end)?;
            tick += delta as f64;
            o = next;
            if o >= track_end {
                break;
            }
            let mut status = d[o];
            if status < 0x80 {
                if running == 0 {
                    return None;
                }
                status = running;
            } else {
                o += 1;
                if status < 0xf0 {
                    running = status;
                }
            }
            if status == 0xff {
                if o >= track_end {
                    return None;
                }
                let meta = d[o];
                o += 1;
                let (len, next) = var_len(d, o, track_end)?;
                o = next;
                if o + len as usize > track_end {
                    return None;
                }
                if meta == 0x51 && len == 3 {
                    tempos.push((tick, ((d[o] as u32) << 16) | ((d[o + 1] as u32) << 8) | d[o + 2] as u32));
                }
                o += len as usize;
                if meta == 0x2f {
                    o = track_end;
                    break;
                }
            } else if status == 0xf0 || status == 0xf7 {
                let (len, next) = var_len(d, o, track_end)?;
                o = next + len as usize;
                if o > track_end {
                    return None;
                }
            } else {
                o += match status {
                    0xf1 | 0xf3 => 1,
                    0xf2 => 2,
                    0xf0..=0xff => 0,
                    _ if matches!(status & 0xf0, 0xc0 | 0xd0) => 1,
                    _ => 2,
                };
                if o > track_end {
                    return None;
                }
            }
        }
        if tick > max_tick {
            max_tick = tick;
        }
    }

    if division & 0x8000 != 0 {
        let fps = (0x100 - ((division >> 8) & 0xff)) as f64;
        let tps = fps * (division & 0xff) as f64;
        return Some(if tps > 0.0 { js_round(max_tick / tps * 1000.0) } else { 0.0 });
    }
    let ppq = if division == 0 { 1.0 } else { division as f64 };
    // stable sort by tick keeps event order for ties
    tempos.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut tempo, mut last, mut total_us) = (500000.0, 0.0, 0.0);
    for &(tick, t) in &tempos {
        if tick < last {
            continue;
        }
        total_us += (tick - last) * tempo / ppq;
        tempo = t as f64;
        last = tick;
    }
    if max_tick > last {
        total_us += (max_tick - last) * tempo / ppq;
    }
    Some(js_round(total_us / 1000.0))
}

/// `Midi.lengths`: per archive 3 file, 0 when missing or unparseable.
pub fn load_lengths(cache: &mut FileStream) -> Vec<f64> {
    (0..cache.file_count(3))
        .map(|i| match cache.read(3, i, true).and_then(|d| parse_midi_length(&d)) {
            Some(len) if len != 0.0 => len,
            _ => {
                tracing::warn!("Failed to parse midi id={i}");
                0.0
            }
        })
        .collect()
}

/// `Midi.getTickLength(id)`.
pub fn tick_length(lengths: &[f64], id: usize) -> i32 {
    (lengths.get(id).copied().unwrap_or(0.0) / 600.0).ceil() as i32 + 1
}
