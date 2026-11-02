//! `tools/pack/**`: the build tooling (`ares-pack`).

pub mod all;
pub mod compiler;
pub mod config;
pub mod dev;
pub mod graphics;
pub mod interface;
pub mod keys;
pub mod map;
pub mod name_map;
pub mod pack_file;
pub mod pix;
pub mod sprite;
pub mod stored_zip;
pub mod tools;
pub mod versionlist;
pub mod worldmap;
pub mod parse;

/// `items.iter().map(f).collect()`, spread over the CPU's threads; results
/// keep `items` order so the serial cache writes that follow stay in order.
pub(crate) fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let f = &f;
    std::thread::scope(|scope| {
        let workers: Vec<_> = items.chunks(items.len().div_ceil(threads).max(1)).map(|chunk| scope.spawn(move || chunk.iter().map(f).collect::<Vec<_>>())).collect();
        workers.into_iter().flat_map(|w| w.join().expect("pack worker")).collect()
    })
}

#[test]
fn par_map_keeps_order() {
    let v: Vec<i32> = (0..1000).collect();
    assert_eq!(par_map(&v, |x| x * 2), v.iter().map(|x| x * 2).collect::<Vec<_>>());
    assert!(par_map(&[] as &[i32], |x| *x).is_empty());
}
