//! A leveled merge-tree index for the log store, driven by the order in
//! which a replay updates the index.
//!
//! The log store keeps one index entry per live record. At full state the
//! index does not fit in a full node's DRAM, so it lives on disk as a
//! leveled merge tree (the RocksDB layout with dynamic level sizes): a
//! write buffer in DRAM, then levels that grow by `ratio` down to a last
//! level holding every entry. What it costs to keep such an index up to
//! date depends on how often an entry is updated again while it still sits
//! in the buffer or an upper level: such an update replaces the entry in
//! place and the older copy never travels further down. That is what the
//! replay measures here; the rest is the standard layout.
//!
//! The model, per level k (0 = the write buffer):
//! - level k holds `C_k` distinct entries. It has been filled by the
//!   updates of the last `w_k` of them, where `w_k` is the window in which
//!   the update stream touches `C_k` distinct ids.
//! - entries leave level k at `C_k / w_k` per update: of the updates in
//!   that window, only one per distinct id survives to move down.
//! - moving `x` entries into level k+1 rewrites the part of level k+1 they
//!   overlap, `x * C_{k+1} / C_k`, as compaction does when it takes key
//!   ranges in turn. A flush from the buffer goes to disk once and is then
//!   merged into the first level, whose whole key range it spans.
//!
//! Relocations (live records the log copies forward when it reclaims
//! space) update entries spread over the whole state, so they are taken
//! to be distinct from everything else: no update of theirs is ever
//! absorbed. Windows longer than the measured stream are not resolved; the
//! distinct share of the longest measured window is used for them, which
//! can only overstate the cost.

use std::collections::HashMap;

/// The fixed choices of the engine. Entry bytes, buffer and ratio are the
/// figures the earlier, assumed engine used.
#[derive(Clone, Copy, Debug)]
pub struct LsmConfig {
    /// Entries in the full index: one per live record of the full state.
    pub entries: u64,
    pub entry_bytes: f64,
    pub buffer_bytes: u64,
    pub ratio: f64,
}

/// One level of the modeled engine.
#[derive(Clone, Debug)]
pub struct LsmLevel {
    /// Distinct entries the level holds.
    pub capacity: f64,
    /// Stream updates over which the level fills: the residence window.
    /// `None` for the last level, which entries never leave.
    pub window: Option<f64>,
    /// Whether `window` lies inside the measured stream.
    pub measured: bool,
    /// Entries moving out of this level per index update (of all kinds).
    pub outflow: f64,
    /// Entries written per index update when that outflow is merged into
    /// the next level.
    pub written: f64,
}

#[derive(Clone, Debug)]
pub struct LsmCost {
    pub levels: Vec<LsmLevel>,
    /// Entries written per index update, all levels: the write amplification.
    pub amplification: f64,
    /// Entries read back per index update by compaction (a flush reads
    /// nothing from disk).
    pub read_amplification: f64,
    /// Relocations per stream update, as given.
    pub relocation_share: f64,
    /// Updates in the measured stream, and distinct ids among them.
    pub stream_updates: u64,
    pub stream_distinct: u64,
    /// Measured distinct share at the windows the levels need, and the
    /// longest window measured.
    pub longest_window: u64,
}

/// Distinct ids per window of the stream, at every power-of-two window
/// length that fits in it: `(window, mean distinct ids per full window)`.
pub fn distinct_curve(stream: &[u64]) -> Vec<(u64, f64)> {
    let n = stream.len();
    if n == 0 {
        return Vec::new();
    }
    // Gap to the previous update of the same id (u32::MAX if none). An
    // update starts a new distinct id in its window when its id was last
    // updated before the window began.
    let mut last: HashMap<u64, u32> = HashMap::with_capacity(n / 4);
    let mut gap: Vec<u32> = Vec::with_capacity(n);
    for (i, id) in stream.iter().enumerate() {
        let i = i as u32;
        gap.push(match last.insert(*id, i) {
            Some(p) => i - p,
            None => u32::MAX,
        });
    }
    drop(last);
    let mut out = Vec::new();
    let mut w = 1usize;
    while w <= n {
        let full = n / w;
        let mut d = 0u64;
        for (i, g) in gap[..full * w].iter().enumerate() {
            if *g as usize > i % w {
                d += 1;
            }
        }
        out.push((w as u64, d as f64 / full as f64));
        w *= 2;
    }
    out
}

/// `distinct_curve`, computed as the updates arrive instead of from a
/// kept stream. Only ids whose hash falls in one of `2^sample_shift`
/// buckets are tracked and their counts scaled back up: a fixed share of
/// the key space, so each window's distinct count is estimated without
/// bias. Memory is one entry per tracked distinct id.
pub struct DistinctCounter {
    sample_shift: u32,
    last: HashMap<u64, u64>,
    /// Updates seen, tracked or not.
    n: u64,
    /// Per power-of-two window k: the window `current` belongs to, distinct
    /// tracked ids in it so far, and the sum over finished windows.
    window: Vec<u64>,
    current: Vec<u64>,
    finished: Vec<u64>,
}

const WINDOW_BITS: usize = 40;

impl DistinctCounter {
    pub fn new(sample_shift: u32) -> Self {
        DistinctCounter {
            sample_shift,
            last: HashMap::new(),
            n: 0,
            window: vec![0; WINDOW_BITS],
            current: vec![0; WINDOW_BITS],
            finished: vec![0; WINDOW_BITS],
        }
    }

    fn tracked(&self, id: u64) -> bool {
        self.sample_shift == 0
            || id.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - self.sample_shift) == 0
    }

    pub fn push(&mut self, id: u64) {
        let i = self.n;
        self.n += 1;
        if !self.tracked(id) {
            return;
        }
        let prev = self.last.insert(id, i);
        for k in 0..WINDOW_BITS {
            let win = i >> k;
            if win != self.window[k] {
                self.finished[k] += self.current[k];
                self.current[k] = 0;
                self.window[k] = win;
            }
            // New to its window if last updated before the window began.
            if prev.is_none_or(|p| p >> k != win) {
                self.current[k] += 1;
            }
        }
    }

    pub fn len(&self) -> u64 {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Mean distinct ids per full window, at each power-of-two window.
    pub fn curve(&self) -> Vec<(u64, f64)> {
        let scale = (1u64 << self.sample_shift) as f64;
        let mut out = Vec::new();
        for k in 0..WINDOW_BITS {
            let full = self.n >> k;
            if full == 0 {
                break;
            }
            // The window `current` counts is finished if it is not the
            // partial one at the end.
            let mut sum = self.finished[k];
            if self.window[k] < full {
                sum += self.current[k];
            }
            out.push((1u64 << k, sum as f64 * scale / full as f64));
        }
        out
    }
}

/// Distinct ids in a window of `w` updates, interpolated on the measured
/// curve (log-log between powers of two). Past the longest window, the
/// longest window's distinct share.
fn distinct_at(curve: &[(u64, f64)], w: f64) -> f64 {
    let (wl, dl) = *curve.last().expect("non-empty curve");
    if w >= wl as f64 {
        return dl / wl as f64 * w;
    }
    if w <= 1.0 {
        return w.max(0.0);
    }
    let i = curve.iter().position(|(cw, _)| *cw as f64 >= w).unwrap();
    let (w0, d0) = curve[i - 1];
    let (w1, d1) = curve[i];
    let t = (w.ln() - (w0 as f64).ln()) / ((w1 as f64).ln() - (w0 as f64).ln());
    (d0.ln() + t * (d1.ln() - d0.ln())).exp()
}

/// Cost of keeping the index on disk, per index update of any kind.
/// `relocations` are extra updates per stream update, never absorbed.
pub fn cost(stream: &[u64], relocations: f64, cfg: LsmConfig) -> LsmCost {
    let curve = distinct_curve(stream);
    cost_from_curve(&curve, stream.len() as u64, relocations, cfg)
}

pub fn cost_from_curve(
    curve: &[(u64, f64)],
    stream_len: u64,
    relocations: f64,
    cfg: LsmConfig,
) -> LsmCost {
    let n = cfg.entries as f64;
    let buffer = cfg.buffer_bytes as f64 / cfg.entry_bytes;
    // Dynamic level sizes: the last level holds everything, each level up
    // is `ratio` times smaller, down to the first one above the buffer.
    let mut caps = vec![n];
    while caps.last().unwrap() / cfg.ratio > buffer {
        caps.push(caps.last().unwrap() / cfg.ratio);
    }
    caps.push(buffer);
    caps.reverse(); // buffer, L1, ..., last
    let longest = curve.last().map_or(0, |c| c.0);
    // Distinct entries (of all kinds) among `w` stream updates.
    let distinct = |w: f64| distinct_at(curve, w) + relocations * w;
    let per_update = 1.0 + relocations;
    let mut levels = Vec::new();
    let (mut amp, mut read_amp) = (0.0, 0.0);
    for k in 0..caps.len() {
        let c = caps[k];
        if k + 1 == caps.len() {
            levels.push(LsmLevel {
                capacity: c,
                window: None,
                measured: true,
                outflow: 0.0,
                written: 0.0,
            });
            break;
        }
        // Smallest window touching c distinct entries (distinct is monotone).
        let (mut lo, mut hi) = (1.0f64, 1.0f64);
        while distinct(hi) < c {
            hi *= 2.0;
        }
        for _ in 0..200 {
            let mid = (lo * hi).sqrt();
            if distinct(mid) < c {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let w = hi;
        // Per index update of any kind.
        let outflow = c / w / per_update;
        let next = caps[k + 1];
        let (written, read) = if k == 0 {
            // Flush once, then merge with the whole first level.
            (outflow * (2.0 + next / c), outflow * (1.0 + next / c))
        } else {
            (outflow * (1.0 + next / c), outflow * (1.0 + next / c))
        };
        amp += written;
        read_amp += read;
        levels.push(LsmLevel {
            capacity: c,
            window: Some(w),
            measured: w <= longest as f64,
            outflow,
            written,
        });
    }
    let stream_distinct = curve
        .last()
        .map_or(0, |c| (c.1 * (stream_len / c.0.max(1)) as f64) as u64);
    LsmCost {
        levels,
        amplification: amp,
        read_amplification: read_amp,
        relocation_share: relocations,
        stream_updates: stream_len,
        stream_distinct: if curve.last().map(|c| c.0) == Some(stream_len) {
            stream_distinct
        } else {
            0
        },
        longest_window: longest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(entries: u64) -> LsmConfig {
        LsmConfig {
            entries,
            entry_bytes: 16.0,
            buffer_bytes: 16 * 100,
            ratio: 10.0,
        }
    }

    #[test]
    fn distinct_curve_counts_tumbling_windows() {
        // a b a b | c c c c
        let s = [1, 2, 1, 2, 3, 3, 3, 3];
        let c = distinct_curve(&s);
        assert_eq!(c[0], (1, 1.0));
        assert_eq!(c[1], (2, 1.5)); // {1,2} {1,2} {3} {3}
        assert_eq!(c[2], (4, 1.5)); // {1,2} {3}
        assert_eq!(c[3], (8, 3.0));
    }

    #[test]
    fn the_running_counter_matches_the_kept_stream() {
        // A skewed stream: a few hot ids among many cold ones.
        let s: Vec<u64> = (0..200_000u64)
            .map(|i| {
                let x = i.wrapping_mul(0x2545_F491_4F6C_DD1D);
                crate::mrc::id64(&(if x % 3 == 0 { x % 97 } else { x % 50_000 }).to_le_bytes())
            })
            .collect();
        let exact = distinct_curve(&s);
        let mut c = DistinctCounter::new(0);
        s.iter().for_each(|id| c.push(*id));
        assert_eq!(c.curve(), exact);
        let mut sampled = DistinctCounter::new(3);
        s.iter().for_each(|id| sampled.push(*id));
        for ((w, d), (w2, d2)) in exact.iter().zip(sampled.curve()) {
            assert_eq!(*w, w2);
            if *w >= 4096 {
                assert!((d2 / d - 1.0).abs() < 0.05, "window {w}: {d2} vs {d}");
            }
        }
    }

    #[test]
    fn a_stream_of_fresh_ids_pays_the_full_layout() {
        // Nothing is absorbed: every level passes all it receives down.
        let s: Vec<u64> = (0..1u64 << 16).collect();
        let c = cost(&s, 0.0, cfg(100_000));
        // buffer 100, levels 1000, 10000, 100000.
        let caps: Vec<f64> = c.levels.iter().map(|l| l.capacity).collect();
        assert_eq!(caps, vec![100.0, 1000.0, 10_000.0, 100_000.0]);
        for l in &c.levels[..3] {
            assert!((l.outflow - 1.0).abs() < 1e-6, "{l:?}");
        }
        // flush 1 + merge (1 + 10) + (1 + 10) + (1 + 10)
        assert!((c.amplification - 34.0).abs() < 1e-3, "{}", c.amplification);
    }

    #[test]
    fn repeated_updates_are_absorbed_above() {
        // 50 ids updated in turn: they hold half the buffer (100) for good,
        // so a flush carries them once per 250 updates, not 250 times.
        let s: Vec<u64> = (0..1u64 << 16).map(|i| i % 50).collect();
        let hot = cost(&s, 0.2, cfg(100_000));
        let fresh: Vec<u64> = (0..1u64 << 16).collect();
        let cold = cost(&fresh, 0.2, cfg(100_000));
        // 100 distinct after 250 updates and 50 relocations: 1/3 per update.
        assert!(
            (hot.levels[0].outflow - 1.0 / 3.0).abs() < 1e-3,
            "{:?}",
            hot.levels[0]
        );
        assert!(
            hot.amplification < 0.6 * cold.amplification,
            "{} vs {}",
            hot.amplification,
            cold.amplification
        );
    }
}
