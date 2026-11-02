//! `::serverstats` (ares only): per-phase
//! cycle timings shown to staff in the `scroll` interface, opened as the main
//! modal. Its text lines (`com_2`..`com_13`; `com_0` is the close button and
//! `com_1` the scroll model) are refreshed every tick with the previous
//! tick's numbers until the modal closes (close button, walking, or the
//! command again).

use super::{Outgoing, World};
use crate::net::server::ServerMessage as M;
use std::collections::VecDeque;

const INTERFACE: &str = "scroll";
/// The text components, top to bottom: `com_2` onwards.
const FIRST_LINE: usize = 2;
const LINE_COUNT: usize = 12;
/// Cycles kept for the average and maximum.
const HISTORY: usize = 100;

/// The `World.cycle()` phases, in order.
pub const PHASES: usize = 11;

#[derive(Default)]
pub struct CycleStats {
    /// The last cycle: per-phase and total milliseconds.
    pub phases: [f64; PHASES],
    pub cycle: f64,
    history: VecDeque<f64>,
    /// Client bytes received since the last cycle started, and during the last one.
    pub bytes_in: usize,
    pub last_in: usize,
    pub last_out: usize,
}

impl CycleStats {
    pub fn record(&mut self, phases: [f64; PHASES], bytes_out: usize) {
        self.phases = phases;
        self.cycle = phases.iter().sum();
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(self.cycle);
        self.last_out = bytes_out;
    }

    fn avg_max(&self) -> (f64, f64) {
        let n = self.history.len().max(1) as f64;
        (
            self.history.iter().sum::<f64>() / n,
            self.history.iter().copied().fold(0.0, f64::max),
        )
    }
}

impl World {
    /// Bytes queued for clients since the runtime last flushed.
    pub(crate) fn outbox_bytes(&self) -> usize {
        self.outbox
            .iter()
            .map(|(_, o)| {
                if let Outgoing::Data(b) = o {
                    b.len()
                } else {
                    0
                }
            })
            .sum()
    }

    /// `::serverstats`: opens the stats scroll, or closes it when open.
    pub(crate) fn toggle_server_stats(&mut self, slot: i32) {
        let id = self.configs.components.get_id(INTERFACE);
        if id == -1 {
            self.message_game(
                slot,
                &format!("::serverstats needs the {INTERFACE} interface."),
            );
            return;
        }
        if self.pl(slot).modal_main == id {
            self.close_modal(slot, true);
            self.pl(slot).server_stats = false;
            return;
        }
        self.open_main_modal(slot, id);
        self.pl(slot).server_stats = true;
    }

    /// Sends the stats to every watcher whose stats scroll is still open
    /// (after `processClientsOut`, so a freshly opened modal arrives first).
    pub(crate) fn update_server_stats(&mut self) {
        let watchers: Vec<i32> = self
            .player_loop()
            .into_iter()
            .filter(|&s| self.player(s).is_some_and(|p| p.server_stats))
            .collect();
        if watchers.is_empty() {
            return;
        }
        let id = self.configs.components.get_id(INTERFACE);
        let components: Vec<i32> = (FIRST_LINE..FIRST_LINE + LINE_COUNT)
            .map(|i| {
                self.configs
                    .components
                    .get_id(&format!("{INTERFACE}:com_{i}"))
            })
            .collect();
        let lines = self.server_stats_lines();
        for slot in watchers {
            let p = self.pl(slot);
            if p.modal_main != id {
                // closed by the player or replaced by a script
                p.server_stats = false;
                continue;
            }
            for (&component, text) in components.iter().zip(&lines) {
                self.write(
                    slot,
                    M::IfSetText {
                        component,
                        text: text.clone(),
                    },
                );
            }
        }
    }

    /// Every 100 ticks with players online: the cycle times to the console
    /// (for headless load tests, e.g. `ares-bots`).
    pub(crate) fn log_server_stats(&self) {
        if self.current_tick % 100 != 0 || self.player_count() == 0 {
            return;
        }
        let (avg, max) = self.stats.avg_max();
        tracing::info!(
            "tick {}: {} players, {} npcs, cycle {:.2}ms (avg {avg:.2}, max {max:.2} over 100 ticks), out {} B/tick",
            self.current_tick,
            self.player_count(),
            self.npcs.count(),
            self.stats.cycle,
            self.stats.last_out
        );
    }

    /// One string per text line of the scroll.
    fn server_stats_lines(&self) -> [String; LINE_COUNT] {
        let s = &self.stats;
        let ph = &s.phases;
        let (avg, max) = s.avg_max();
        let colour = match s.cycle / self.tick_rate {
            r if r < 0.25 => "@gre@",
            r if r < 0.75 => "@yel@",
            _ => "@red@",
        };
        let up = self.started.elapsed().as_secs();
        [
            format!("Server stats - tick {}", self.current_tick),
            format!("{colour}Cycle {:.2}ms of {}ms", s.cycle, self.tick_rate),
            format!("Last 100 ticks: avg {avg:.2}ms, max {max:.2}ms"),
            format!("World {:.2}ms, Clients in {:.2}ms", ph[0], ph[1]),
            format!("Npc events {:.2}ms, Npcs {:.2}ms", ph[2], ph[3]),
            format!("Players {:.2}ms", ph[4]),
            format!("Logouts {:.2}ms, Logins {:.2}ms", ph[5], ph[6]),
            format!("Zones {:.2}ms, Info {:.2}ms", ph[7], ph[8]),
            format!("Clients out {:.2}ms, Cleanup {:.2}ms", ph[9], ph[10]),
            format!(
                "{} players, {} npcs",
                self.player_count(),
                self.npcs.count()
            ),
            format!("Net in {} B, out {} B per tick", s.last_in, s.last_out),
            format!("Uptime {}:{:02}:{:02}", up / 3600, up / 60 % 60, up % 60),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_keeps_the_last_hundred() {
        let mut s = CycleStats::default();
        for i in 0..150 {
            s.record(
                [i as f64, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                0,
            );
        }
        assert_eq!(s.cycle, 150.0);
        let (avg, max) = s.avg_max();
        assert_eq!(max, 150.0);
        assert_eq!(avg, (50..150).map(|i| i as f64 + 1.0).sum::<f64>() / 100.0);
    }
}
