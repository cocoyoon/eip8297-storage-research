//! Phase-2 harness: one workload, several physical backends, commitment
//! held fixed at PBT. Sweeps the paged/append cut depth to locate `d*`.

use crate::gen::{Params, World, Write as StateWrite};
use crate::mainnet;
use pbt_ref::tree::{CommitSink, PbtTree, ReadPath, RecordKind, RewriteStats, ZoneLabel};
use pbt_store::append::BranchHashPolicy;
use pbt_store::btree::BtreeBackend;
use pbt_store::mrc::CacheProfile;
use pbt_store::paged::Packing;
use pbt_store::shape::SharedShape;
use pbt_store::{append::AppendBackend, kv::KvBackend, paged::PagedBackend, split::SplitBackend};
use pbt_store::{Backend, DeviceProfile, Metrics, PROFILES};
use std::fmt::Write as _;
use std::time::Instant;

pub struct BenchConfig {
    pub params: Params,
    pub cuts: Vec<u32>,
    pub resident_levels: u32,
    pub reads_per_block: usize,
    /// true = model reads as EVM value lookups; false = witness/proof walks.
    pub value_reads: bool,
    /// Witness reads only: true = the block's lookups are resolved as ONE
    /// multiproof (each distinct record/page fetched once for the batch),
    /// false = each lookup served independently. Real witness generation
    /// batches; the independent path is kept so both can be reported.
    pub batched_reads: bool,
    pub with_kv: bool,
    pub real_io_dir: Option<String>,
    /// Blocks run before the measured window. They warm whatever state a
    /// backend carries across blocks and are excluded from every figure.
    pub warmup_blocks: usize,
    /// Profile a bounded cache on the write path of both disciplines and
    /// compare them under an equal DRAM grant (O7).
    pub cache_sweep: bool,
    /// DRAM budgets to tabulate, in MB. Empty = a grid derived from the run.
    pub dram_mb: Vec<f64>,
    /// Set when the workload is a mainnet replay: the full state it stands
    /// in for, and what it was built from.
    pub mainnet: Option<MainnetCtx>,
    /// Write every committed record and delete, the starting state first,
    /// to this file (`stream.rs`), for a real engine to replay.
    pub record_stream: Option<String>,
}

pub struct MainnetCtx {
    pub shape: SharedShape,
    pub scale: mainnet::Scale,
    pub facts: mainnet::ReplayFacts,
}

/// Where a run's state writes come from.
pub trait Workload {
    fn describe(&self) -> String;
    /// The starting state, as batches committed one after another. One
    /// batch is the original behaviour; several bound the memory a very
    /// large starting state needs to commit.
    fn genesis(&mut self) -> Vec<Vec<StateWrite>>;
    fn block(&mut self) -> Vec<StateWrite>;
    /// The execution reads of the block `block` just returned, served
    /// before its commit. Default: none.
    fn block_reads(&mut self) -> Vec<mainnet::StateRead> {
        Vec::new()
    }
}

impl Workload for World {
    fn describe(&self) -> String {
        let p = &self.params;
        format!(
            "seed {}, {} EOAs, {} contracts ({} mega), {} ops/block, contract Zipf s={}, EOA access {}",
            p.seed,
            p.eoas,
            p.contracts,
            p.mega_contracts,
            p.ops_per_block,
            p.zipf_s,
            if p.eoa_zipf_s > 0.0 { format!("Zipf s={}", p.eoa_zipf_s) } else { "uniform".to_string() }
        )
    }
    fn genesis(&mut self) -> Vec<Vec<StateWrite>> {
        vec![World::genesis(self)]
    }
    fn block(&mut self) -> Vec<StateWrite> {
        World::block(self)
    }
}

/// Branch records on the path of each leaf written in the measured window.
#[derive(Default, Clone)]
struct LeafDepths {
    /// zone name -> (leaves written, sum of node depths, deepest)
    zones: std::collections::BTreeMap<String, (u64, u64, u32)>,
}

struct Row {
    name: String,
    m: Metrics,
    extra: String,
    cache: Option<CacheProfile>,
    /// Page store only: the curve with its value store sharing the cache.
    cache_vs: Option<CacheProfile>,
    /// Log store in a mainnet replay: distinct ids per window of its index
    /// update stream, and the stream's length.
    index_curve: Option<(Vec<(u64, f64)>, u64)>,
}

pub struct BenchOutput {
    pub md: String,
    /// Full bounded-cache curves, when `cache_sweep` is on.
    pub mrc_csv: Option<String>,
}

/// What the generator was asked for, recorded in the report because the
/// bounded-cache result depends on it.
struct RunFacts {
    blocks: u64,
    warmup: u64,
    workload: String,
    /// Lookups were value reads (the default), not witness walks.
    value_reads: bool,
    leaf_depths: LeafDepths,
    /// Execution reads served in the measured window: existing, absent.
    exec_reads: (u64, u64),
}

pub fn run(mut cfg: BenchConfig) -> BenchOutput {
    let blocks = cfg.params.blocks;
    let world = World::new(std::mem::take(&mut cfg.params));
    run_with(cfg, blocks, Box::new(world))
}

/// Run `blocks` measured blocks of `world` (after the configured warm-up)
/// through every backend.
pub fn run_with(cfg: BenchConfig, blocks: usize, mut world: Box<dyn Workload>) -> BenchOutput {
    let mut facts = RunFacts {
        blocks: blocks as u64,
        warmup: cfg.warmup_blocks as u64,
        value_reads: cfg.value_reads,
        workload: world.describe(),
        leaf_depths: LeafDepths::default(),
        exec_reads: (0, 0),
    };

    // Build backends. Each gets the identical record stream.
    let mut backends: Vec<Box<dyn Backend>> = Vec::new();
    if let Some(mn) = &cfg.mainnet {
        // A replay stands in for a state far larger than itself, so only
        // the stores that can be judged against that full state run: the
        // page store told what the full state looks like, the log store
        // whose branch reads are profiled, and the modeled B+tree of
        // records placed by that same full state.
        backends.push(Box::new(
            PagedBackend::with_packing(None, cfg.resident_levels, Packing::Positional)
                .with_shape(mn.shape.clone()),
        ));
        backends.push(Box::new(AppendBackend::with_policy(
            None,
            cfg.resident_levels * 6,
            BranchHashPolicy::ReadBack,
        )));
        backends.push(Box::new(BtreeBackend::new(
            mn.shape.clone(),
            2 * mn.scale.leaves(),
            cfg.resident_levels,
        )));
        // The page store is charged the read of every value-store page it
        // overwrites. The log store gets the same event charged: the read
        // of the value a transaction is about to change.
        for b in backends.iter_mut() {
            b.charge_old_value_reads();
        }
    }
    if cfg.mainnet.is_none() {
        if cfg.with_kv {
            let dir = cfg
                .real_io_dir
                .as_deref()
                .map(|d| format!("{d}/mdbx"))
                .unwrap_or_else(|| "/tmp/pbt-mdbx".into());
            backends.push(Box::new(KvBackend::new(&dir)));
        }
        let paged_wal = cfg
            .real_io_dir
            .as_deref()
            .map(|d| format!("{d}/paged_pos.wal"));
        backends.push(Box::new(PagedBackend::with_packing(
            paged_wal.as_deref(),
            cfg.resident_levels,
            Packing::Positional,
        )));
        let paged_wal2 = cfg
            .real_io_dir
            .as_deref()
            .map(|d| format!("{d}/paged_struct.wal"));
        backends.push(Box::new(PagedBackend::with_packing(
            paged_wal2.as_deref(),
            cfg.resident_levels,
            Packing::Structural,
        )));
        let append_log = cfg
            .real_io_dir
            .as_deref()
            .map(|d| format!("{d}/append.log"));
        for pol in [
            BranchHashPolicy::Free,
            BranchHashPolicy::ReadBack,
            BranchHashPolicy::CacheInDram,
        ] {
            let path = cfg
                .real_io_dir
                .as_deref()
                .map(|d| format!("{d}/append_{pol:?}.log"));
            backends.push(Box::new(AppendBackend::with_policy(
                path.as_deref(),
                cfg.resident_levels * 6,
                pol,
            )));
        }
        let _ = &append_log;
        for &cut in &cfg.cuts {
            backends.push(Box::new(SplitBackend::new(cut, cfg.resident_levels)));
        }
    }

    // Genesis: build the tree and prime every backend with the same batches.
    let mut tree = PbtTree::new();
    let mut stream = cfg.record_stream.as_deref().map(|p| {
        let mut w = crate::stream::StreamWriter::create(p).expect("record stream file");
        w.genesis().unwrap();
        w
    });
    if cfg.cache_sweep {
        for b in backends.iter_mut() {
            b.prepare_cache_sweep();
        }
    }
    let genesis = world.genesis();
    let n_batches = genesis.len();
    for (i, batch) in genesis.into_iter().enumerate() {
        for (k, v) in &batch {
            tree.write(k, v);
        }
        drop(batch);
        let mut sink = CommitSink::default();
        let mut stats = RewriteStats::default();
        tree.commit(&mut stats, Some(&mut sink));
        if let Some(w) = stream.as_mut() {
            w.sink(&sink).unwrap();
        }
        eprintln!(
            "[bench] genesis batch {}/{}: {} records, {}",
            i + 1,
            n_batches,
            sink.records.len(),
            crate::alloc::heap()
        );
        for b in backends.iter_mut() {
            b.commit_block(&sink);
        }
    }
    if cfg.cache_sweep {
        for b in backends.iter_mut() {
            b.enable_cache_profile();
            // Update streams from here on drive the merge-tree models: the
            // log store's index entries, the page store's pages.
            if cfg.mainnet.is_some() {
                b.record_index_stream();
            }
        }
    }
    // Genesis and warm-up IO are setup, not measurement: the baseline is
    // taken here and again when the measured window opens.
    let mut baseline: Vec<Metrics> = backends.iter().map(|b| b.metrics()).collect();

    let t0 = Instant::now();
    let mut read_keys: Vec<Vec<u8>> = Vec::new();
    let mut paths: Vec<ReadPath> = Vec::new();
    let mut read_paths_total: u64 = 0;
    let warmup = cfg.warmup_blocks;
    if let Some(w) = stream.as_mut() {
        w.start().unwrap();
    }
    for i in 0..warmup + blocks {
        if i == warmup {
            baseline = backends.iter().map(|b| b.metrics()).collect();
            for b in backends.iter_mut() {
                b.begin_measurement();
            }
            read_paths_total = 0;
        }
        let writes = world.block();
        // Execution comes before the commit: its reads meet the cache first.
        for (k, present) in world.block_reads() {
            if i >= warmup {
                if present {
                    facts.exec_reads.0 += 1;
                } else {
                    facts.exec_reads.1 += 1;
                }
            }
            for b in backends.iter_mut() {
                b.serve_state_read(&k, present);
            }
        }
        // Sample read keys from this block's traffic (hot-biased, as reads are).
        read_keys.clear();
        for (k, _) in writes.iter().take(cfg.reads_per_block) {
            read_keys.push(k.clone());
        }
        for (k, v) in &writes {
            tree.write(k, v);
        }
        let mut sink = CommitSink::default();
        let mut stats = RewriteStats::default();
        tree.commit(&mut stats, Some(&mut sink));
        if let Some(w) = stream.as_mut() {
            w.block(i as u32).unwrap();
            w.sink(&sink).unwrap();
        }
        if cfg.cache_sweep && i >= warmup {
            for r in sink.records.iter().filter(|r| r.kind == RecordKind::Leaf) {
                let zone = match r.zone {
                    ZoneLabel::Account => "account",
                    ZoneLabel::Code => "code",
                    ZoneLabel::Storage => "storage",
                    _ => "other",
                };
                let e = facts.leaf_depths.zones.entry(zone.to_string()).or_default();
                e.0 += 1;
                e.1 += r.node_depth as u64;
                e.2 = e.2.max(r.node_depth);
            }
        }
        for b in backends.iter_mut() {
            b.commit_block(&sink);
        }
        // Resolve every read key against the tree FIRST, so a batched
        // backend sees the whole block's read set at once.
        paths.clear();
        for k in &read_keys {
            if let Some(rp) = tree.read_path(k) {
                paths.push(rp);
            }
        }
        read_paths_total += paths.len() as u64;
        for b in backends.iter_mut() {
            if cfg.value_reads {
                for rp in &paths {
                    b.serve_value_read(rp);
                }
            } else if cfg.batched_reads {
                b.serve_read_batch(&paths);
            } else {
                for rp in &paths {
                    b.serve_read(rp);
                }
            }
        }
        if (i + 1) % 200 == 0 {
            eprintln!(
                "[bench] block {}/{} ({:.1?}), {}",
                i + 1,
                warmup + blocks,
                t0.elapsed(),
                crate::alloc::heap()
            );
        }
    }
    if let Some(w) = stream.take() {
        w.finish().unwrap();
    }
    for b in backends.iter_mut() {
        b.finalize();
    }
    eprintln!(
        "[bench] reads: requested {}/blk, resolved {:.1}/blk ({} total), mode={}",
        cfg.reads_per_block,
        read_paths_total as f64 / blocks.max(1) as f64,
        read_paths_total,
        if cfg.value_reads {
            "value"
        } else if cfg.batched_reads {
            "witness-batched"
        } else {
            "witness-per-key"
        }
    );

    let rows: Vec<Row> = backends
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let m = b.metrics();
            let base = &baseline[i];
            Row {
                name: b.name(),
                m: Metrics {
                    seq_bytes_written: m.seq_bytes_written.saturating_sub(base.seq_bytes_written),
                    seq_bytes_read: m.seq_bytes_read.saturating_sub(base.seq_bytes_read),
                    rand_page_writes: m.rand_page_writes.saturating_sub(base.rand_page_writes),
                    page_reads: m.page_reads.saturating_sub(base.page_reads),
                    read_bytes: m.read_bytes.saturating_sub(base.read_bytes),
                    fsyncs: m.fsyncs.saturating_sub(base.fsyncs),
                    live_bytes: m.live_bytes,
                    garbage_bytes: m.garbage_bytes,
                    compactions: m.compactions,
                    record_writes: m.record_writes.saturating_sub(base.record_writes),
                    record_deletes: m.record_deletes.saturating_sub(base.record_deletes),
                    reads_served: m.reads_served.saturating_sub(base.reads_served),
                    wall_ns: m.wall_ns.saturating_sub(base.wall_ns),
                    disk_bytes: m.disk_bytes,
                    dram_bytes: m.dram_bytes,
                },
                extra: b.extra_report(),
                cache: b.cache_profile(),
                cache_vs: b.value_store_profile(),
                index_curve: b.index_curve(),
            }
        })
        .collect();
    baseline.clear();

    let mut md = report(&rows, blocks as u64, &cfg.cuts, cfg.resident_levels);
    let mut mrc_csv = None;
    if let Some(mn) = &cfg.mainnet {
        let (section, csv) = mainnet_report(&rows, &facts, mn);
        md.push_str(&section);
        mrc_csv = Some(csv);
    } else if cfg.cache_sweep {
        md.push_str(&cache_sweep_report(&rows, &facts, &cfg.dram_mb));
        md.push_str(&low_dram_report(&rows, &facts));
        mrc_csv = Some(cache_sweep_csv(&rows, facts.blocks));
    }
    BenchOutput { md, mrc_csv }
}

const MB: f64 = (1u64 << 20) as f64;

/// Which end of a curve's uncertainty to take above its exact range.
#[derive(Clone, Copy, PartialEq)]
enum Bound {
    Lo,
    Hi,
}

/// How reads are priced: per IO, or by fixed overhead plus bytes moved.
/// The two bracket the truth (draft Appendix C) and disagree here, because
/// a log read moves one sector and a page read moves 4 KB.
#[derive(Clone, Copy, PartialEq)]
enum Price {
    PerIo,
    Bandwidth,
}

/// Extra write-path work charged to a row as a sensitivity, per window.
#[derive(Clone, Copy, Default)]
struct Extra {
    reads: u64,
    rand_writes: u64,
}

struct AtBudget {
    cap: u64,
    reads_lo: u64,
    reads_hi: u64,
    m: Metrics,
}

/// A row's metrics as they would be with `dram` bytes in total: the fixed
/// part paid first, the rest spent on an LRU over its write-path reads.
/// None when the fixed part alone exceeds the budget.
fn at_budget(
    m: &Metrics,
    cp: &CacheProfile,
    dram: u64,
    bound: Bound,
    extra: Extra,
) -> Option<AtBudget> {
    let cap = cp.capacity_for(dram)?;
    let (lo, hi) = cp.bounds_at(cap);
    let misses = if bound == Bound::Lo { lo } else { hi };
    let per_read = if cp.accesses > 0 {
        cp.read_bytes as f64 / cp.accesses as f64
    } else {
        0.0
    };
    let mut a = m.clone();
    a.page_reads = m.page_reads - cp.accesses + misses + extra.reads;
    a.read_bytes = m.read_bytes - cp.read_bytes
        + (misses as f64 * per_read) as u64
        + extra.reads * pbt_store::PAGE;
    a.rand_page_writes += extra.rand_writes;
    // Both sides hold the same budget under a hard ceiling, so DRAM is not
    // priced: zeroing it makes the bandwidth cost an IO-only figure too.
    a.dram_bytes = 0;
    Some(AtBudget {
        cap,
        reads_lo: lo,
        reads_hi: hi,
        m: a,
    })
}

fn io_cost(m: &Metrics, p: &DeviceProfile, blocks: u64, price: Price) -> f64 {
    match price {
        Price::PerIo => m.cost(p) / blocks.max(1) as f64,
        Price::Bandwidth => m.cost_per_block_bw(p, blocks),
    }
}

fn profiled<'a>(rows: &'a [Row], prefix: &str) -> Option<(&'a Row, &'a CacheProfile)> {
    rows.iter()
        .find(|r| r.name.starts_with(prefix))
        .and_then(|r| r.cache.as_ref().map(|c| (r, c)))
}

/// Smallest budget in [lo, hi] at which the log store is no more expensive
/// than the page store, on a fine grid. `Some(lo)` means it already wins at
/// the bottom of the range; `None` means it never does.
fn crossover_scan(
    lo: u64,
    hi: u64,
    log: impl Fn(u64) -> Option<f64>,
    page: impl Fn(u64) -> Option<f64>,
) -> Option<u64> {
    let hi = hi.max(lo);
    let steps = 4000u64;
    for i in 0..=steps {
        let d = lo + (hi - lo) * i / steps;
        let (Some(a), Some(p)) = (log(d), page(d)) else {
            continue;
        };
        if a <= p {
            return Some(d);
        }
    }
    None
}

/// Crossover of the two base models, from the log store's fixed part up to
/// whichever store is fully resident last.
#[allow(clippy::too_many_arguments)]
fn crossover(
    am: &Metrics,
    ac: &CacheProfile,
    pm: &Metrics,
    pc: &CacheProfile,
    prof: &DeviceProfile,
    blocks: u64,
    price: Price,
    log_bound: Bound,
    page_bound: Bound,
) -> Option<u64> {
    crossover_scan(
        ac.fixed_dram_bytes,
        ac.full_dram_bytes().max(pc.full_dram_bytes()),
        |d| {
            at_budget(am, ac, d, log_bound, Extra::default())
                .map(|x| io_cost(&x.m, prof, blocks, price))
        },
        |d| {
            at_budget(pm, pc, d, page_bound, Extra::default())
                .map(|x| io_cost(&x.m, prof, blocks, price))
        },
    )
}

/// The page store at a DRAM budget WITH its value store, which the base
/// model omits. Mechanism as read from NOMT's source (c3c0e55): the value
/// store's leaf pages share the cache with merkle pages; every dirtied leaf
/// page is read if uncached and rewritten copy-on-write on every commit;
/// the bottom branch node above a rewritten leaf is rewritten too and all
/// such nodes stay resident; both files write free-list pages; and a commit
/// fsyncs five files, not the three the merkle side alone needs.
///
/// MEASURED on a modeled leaf layer: reads (one curve over both kinds of
/// page), distinct leaf pages dirtied, distinct bottom branch nodes above
/// them. ASSUMED: four free-list page writes per commit. Copy-on-write
/// pages are priced as random writes: NOMT pops its free list last-in
/// first-out with no sorting, and a replay of that allocator over this
/// workload gave runs averaging 1.04 pages.
///
/// `vs` may carry extra fixed DRAM (the commit working set); `pc` is the
/// merkle-only curve, used only to take the merkle reads back out of `m`.
fn page_with_value_store(
    m: &Metrics,
    pc: &CacheProfile,
    vs: &CacheProfile,
    dram: u64,
    blocks: u64,
    bound: Bound,
) -> Option<AtBudget> {
    let cap = vs.capacity_for(dram)?;
    let (lo, hi) = vs.bounds_at(cap);
    let misses = if bound == Bound::Lo { lo } else { hi };
    let b = blocks.max(1);
    let mut a = m.clone();
    // `m` holds merkle read-modify-write reads plus value-lookup reads. The
    // shared cache's misses replace the former. The lookups were charged a
    // leaf and one interior miss each; with every bottom branch node
    // resident the interior miss does not happen.
    let lookup_reads = m.page_reads - pc.accesses;
    let interior = lookup_reads.saturating_sub(m.reads_served);
    a.page_reads = lookup_reads - interior + misses;
    a.read_bytes = (lookup_reads - interior + misses) * pbt_store::PAGE;
    a.rand_page_writes += vs.value_leaf_page_writes + vs.value_bbn_writes + 4 * b;
    a.fsyncs += 2 * b;
    a.dram_bytes = 0;
    Some(AtBudget {
        cap,
        reads_lo: lo,
        reads_hi: hi,
        m: a,
    })
}

/// "X MB (N cached branches, P% of live)" for a crossover budget.
fn describe_crossover(d: Option<u64>, ac: &CacheProfile) -> String {
    match d {
        None => "none in range".to_string(),
        Some(d) => {
            let cap = ac.capacity_for(d).unwrap_or(0);
            format!(
                "{:.1} MB ({} cached branches, {:.1}% of live)",
                d as f64 / MB,
                cap,
                100.0 * cap as f64 / ac.cacheable_entries.max(1) as f64
            )
        }
    }
}

fn range(lo: f64, hi: f64) -> String {
    if (hi - lo).abs() < 0.05 {
        format!("{hi:.1}")
    } else {
        format!("{lo:.1}-{hi:.1}")
    }
}

fn cache_sweep_report(rows: &[Row], facts: &RunFacts, dram_mb: &[f64]) -> String {
    let mut md = String::new();
    let (Some((ar, ac)), Some((pr, pc))) = (
        profiled(rows, "append-readback"),
        profiled(rows, "paged-pos"),
    ) else {
        return md;
    };
    let blocks = facts.blocks;
    let b = blocks.max(1) as f64;
    let none = Extra::default();

    let _ = writeln!(md, "\n## Bounded cache under an equal DRAM grant (O7)\n");
    let _ = writeln!(
        md,
        "> Nature: SYNTHETIC workload, COUNTER model. No cache exists; the figures are\n> what an LRU of each size would have missed, from one pass that records every\n> write-path read's stack distance. Both disciplines are given the SAME total\n> DRAM. The log store pays its index and resident top out of that budget first\n> and spends the rest on whole branch records; the page store spends it on 4 KB\n> pages. Only write-path reads are cached here; lookups are unchanged. Wall\n> time in a run with this section on includes the profiler and must not be cited.\n"
    );
    let _ = writeln!(md, "- workload: {}", facts.workload);
    let _ = writeln!(
        md,
        "- window: {} measured blocks after {} warm-up blocks",
        facts.blocks, facts.warmup
    );
    let _ = writeln!(
        md,
        "- log store (`{}`): fixed {:.1} MB, of which the index is {} entries at {} B (ASSUMED) = {:.1} MB; {} cacheable branches at {} B each (70 B record + 16 B ASSUMED bookkeeping); everything resident at {:.1} MB; {:.1} write-path reads/blk with no cache, {:.1}% of them cold",
        ar.name,
        ac.fixed_dram_bytes as f64 / MB,
        ac.index_entries,
        ac.index_entry_bytes,
        ac.index_entries as f64 * ac.index_entry_bytes / MB,
        ac.cacheable_entries,
        ac.entry_bytes,
        ac.full_dram_bytes() as f64 / MB,
        ac.accesses as f64 / b,
        if ac.accesses > 0 { 100.0 * ac.cold as f64 / ac.accesses as f64 } else { 0.0 },
    );
    let _ = writeln!(
        md,
        "- page store (`{}`): fixed {:.1} MB, no index; {} cacheable pages at {} B each; everything resident at {:.1} MB; {:.1} write-path reads/blk with no cache, {:.1}% of them cold; {:.1} elided pages touched per block at no charge (rebuilding one reads the value store, which this base model omits)",
        pr.name,
        pc.fixed_dram_bytes as f64 / MB,
        pc.cacheable_entries,
        pc.entry_bytes,
        pc.full_dram_bytes() as f64 / MB,
        pc.accesses as f64 / b,
        if pc.accesses > 0 { 100.0 * pc.cold as f64 / pc.accesses as f64 } else { 0.0 },
        pc.uncharged_elided_touches as f64 / b,
    );
    let _ = writeln!(
        md,
        "- exact range: a cold read is one of an entry untouched since profiling began. The curve is exact up to {} cached branches for the log store and {} cached pages for the page store (the stack sizes when the window opened). Above that a cold read is an unknown, so the figure is a range `lo-hi`: `lo` counts it as a hit, which is what a cache holding the entry from the start would do, and `hi` counts it as a miss.\n",
        ac.exact_upto, pc.exact_upto
    );

    // Budgets: explicit, or spread across the log store's cache span plus
    // two points below its index, where it cannot run at all.
    let mut budgets: Vec<u64> = if dram_mb.is_empty() {
        let fixed = ac.fixed_dram_bytes as f64;
        let span = (ac.full_dram_bytes() - ac.fixed_dram_bytes) as f64;
        let mut v: Vec<u64> = [0.25, 0.5].iter().map(|f| (fixed * f) as u64).collect();
        for f in [
            0.0, 0.01, 0.02, 0.05, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.75, 0.9, 1.0,
        ] {
            v.push((fixed + span * f).ceil() as u64);
        }
        v
    } else {
        dram_mb.iter().map(|m| (m * MB) as u64).collect()
    };
    budgets.sort_unstable();
    budgets.dedup();

    let _ = writeln!(md, "### Write-path reads per block\n");
    let _ = writeln!(
        md,
        "| DRAM MB | log: cached branches | % of live | log reads/blk | page: cached pages | % of stored | page reads/blk |"
    );
    let _ = writeln!(md, "|---|---|---|---|---|---|---|");
    for &d in &budgets {
        let cell = |m: &Metrics, c: &CacheProfile| match at_budget(m, c, d, Bound::Hi, none) {
            Some(x) => format!(
                "{} | {:.1}% | {}",
                x.cap,
                100.0 * x.cap as f64 / c.cacheable_entries.max(1) as f64,
                range(x.reads_lo as f64 / b, x.reads_hi as f64 / b)
            ),
            None => "index does not fit | - | -".to_string(),
        };
        let _ = writeln!(
            md,
            "| {:.1} | {} | {} |",
            d as f64 / MB,
            cell(&ar.m, ac),
            cell(&pr.m, pc)
        );
    }

    for prof in PROFILES.iter() {
        let _ = writeln!(
            md,
            "\n### IO cost per block at equal DRAM, profile `{}`\n",
            prof.name
        );
        let _ = writeln!(
            md,
            "DRAM is not priced: both sides hold the same budget. Reads are priced two ways, per IO and by bytes moved; a log read moves one sector and a page read 4 KB, so the two differ. Ratios use the `hi` end of both.\n"
        );
        let _ = writeln!(
            md,
            "| DRAM MB | log, per IO | page, per IO | page / log | log, by bytes | page, by bytes | page / log |"
        );
        let _ = writeln!(md, "|---|---|---|---|---|---|---|");
        for &d in &budgets {
            let mut cells: Vec<String> = Vec::new();
            for price in [Price::PerIo, Price::Bandwidth] {
                let cost = |m: &Metrics, c: &CacheProfile| -> Option<(f64, f64)> {
                    let lo = at_budget(m, c, d, Bound::Lo, none)?;
                    let hi = at_budget(m, c, d, Bound::Hi, none)?;
                    Some((
                        io_cost(&lo.m, prof, blocks, price),
                        io_cost(&hi.m, prof, blocks, price),
                    ))
                };
                let a = cost(&ar.m, ac);
                let p = cost(&pr.m, pc);
                cells.push(a.map_or("-".to_string(), |(lo, hi)| range(lo, hi)));
                cells.push(p.map_or("-".to_string(), |(lo, hi)| range(lo, hi)));
                cells.push(match (a, p) {
                    (Some((_, ca)), Some((_, cp))) => format!("{:.2}x", cp / ca.max(1e-9)),
                    _ => "-".to_string(),
                });
            }
            let _ = writeln!(md, "| {:.1} | {} |", d as f64 / MB, cells.join(" | "));
        }
        let _ = writeln!(md);
        for (price, label) in [(Price::PerIo, "per IO"), (Price::Bandwidth, "by bytes")] {
            // Worst case for the log store, then best case: the true
            // crossover lies between them.
            let worst = crossover(
                &ar.m,
                ac,
                &pr.m,
                pc,
                prof,
                blocks,
                price,
                Bound::Hi,
                Bound::Lo,
            );
            let best = crossover(
                &ar.m,
                ac,
                &pr.m,
                pc,
                prof,
                blocks,
                price,
                Bound::Lo,
                Bound::Hi,
            );
            let text = match (best, worst) {
                (Some(d), _) if d == ac.fixed_dram_bytes => format!(
                    "none needed: the log store is already cheaper at {:.1} MB, its index and resident top with no branch cache",
                    d as f64 / MB
                ),
                (Some(lo), Some(hi)) if lo == hi => format!(
                    "the log store becomes cheaper at **{}**; below that the page store is cheaper",
                    describe_crossover(Some(hi), ac)
                ),
                (lo, hi) => format!(
                    "the log store becomes cheaper somewhere between {} and {}; below that the page store is cheaper",
                    describe_crossover(lo, ac),
                    describe_crossover(hi, ac)
                ),
            };
            let _ = writeln!(md, "- crossover, reads priced {label}: {text}");
        }
    }

    // What the crossover depends on that this run did not measure.
    let _ = writeln!(md, "\n### What moves the crossover\n");
    let _ = writeln!(
        md,
        "Each row changes ONE thing and reports where the log store becomes cheaper (worst case for the log store above the exact range). The second row gives the page store the value store its base model omits: leaf pages share the cache with merkle pages, every dirtied leaf page is rewritten copy-on-write, and a commit takes five fsyncs, not three. That mechanism is from NOMT's source (c3c0e55); the leaf-page and branch-node counts are measured on a modeled value store. The log store in every row keeps its index in DRAM. The remaining rows change how the log store's curve maps onto DRAM.\n"
    );
    let _ = writeln!(
        md,
        "| change | {}, per IO | {}, by bytes | {}, per IO | {}, by bytes |",
        PROFILES[0].name, PROFILES[0].name, PROFILES[1].name, PROFILES[1].name
    );
    let _ = writeln!(md, "|---|---|---|---|---|");
    let pvs = pr.cache_vs.as_ref();
    let mut variants: Vec<(String, CacheProfile, bool)> =
        vec![("as modeled".to_string(), ac.clone(), false)];
    if pvs.is_some() {
        variants.push((
            "page store with its value store".to_string(),
            ac.clone(),
            true,
        ));
    }
    variants.push((
        "log store index at 8 B per entry".to_string(),
        ac.with_index_entry_bytes(8.0),
        false,
    ));
    variants.push((
        "log store index at 24 B per entry".to_string(),
        ac.with_index_entry_bytes(24.0),
        false,
    ));
    variants.push((
        "log store index at 32 B per entry".to_string(),
        ac.with_index_entry_bytes(32.0),
        false,
    ));
    variants.push((
        "log store cache entry at 128 B".to_string(),
        ac.with_entry_bytes(128),
        false,
    ));
    for (label, a, with_value_store) in &variants {
        let mut cells: Vec<String> = Vec::new();
        for prof in PROFILES.iter() {
            for price in [Price::PerIo, Price::Bandwidth] {
                let d = crossover_scan(
                    a.fixed_dram_bytes,
                    a.full_dram_bytes().max(pc.full_dram_bytes()),
                    |d| {
                        at_budget(&ar.m, a, d, Bound::Hi, none)
                            .map(|x| io_cost(&x.m, prof, blocks, price))
                    },
                    |d| match (with_value_store, pvs) {
                        (true, Some(vs)) => {
                            page_with_value_store(&pr.m, pc, vs, d, blocks, Bound::Hi)
                                .map(|x| io_cost(&x.m, prof, blocks, price))
                        }
                        _ => at_budget(&pr.m, pc, d, Bound::Lo, none)
                            .map(|x| io_cost(&x.m, prof, blocks, price)),
                    },
                );
                cells.push(match d {
                    Some(d) if d == a.fixed_dram_bytes => {
                        format!("{:.1} MB (no cache needed)", d as f64 / MB)
                    }
                    other => describe_crossover(other, a),
                });
            }
        }
        let _ = writeln!(md, "| {} | {} |", label, cells.join(" | "));
    }
    md
}

/// A log store whose index is NOT held in DRAM. Every field is an
/// ASSUMPTION about an index engine this crate does not implement; the
/// counts they multiply are measured.
#[derive(Clone, Copy)]
struct DiskIndex {
    /// DRAM kept per index entry: a filter or fingerprint, plus two bits to
    /// record that a logged copy is dead.
    resident_bytes: f64,
    /// DRAM per cached branch. A cached record must also remember where its
    /// logged copy is, so that rewriting it can mark that copy dead without
    /// a lookup.
    cache_entry_bytes: u64,
    /// Random 4 KB reads per lookup the DRAM part cannot answer.
    reads_per_lookup: u64,
    /// Sequential bytes per index update, written and, because an index
    /// that amplifies writes by merging reads what it rewrites, read.
    bytes_per_update: f64,
    /// DRAM the index engine holds whatever the cache size: its write
    /// buffer and the pointers that take a lookup to the right page.
    fixed_bytes: u64,
}

/// The log store at a DRAM budget with its index on disk. `ac` may carry
/// extra fixed DRAM. With the DRAM figures of the base model and nothing
/// charged per lookup or update this is `at_budget` plus the displaced-leaf
/// fetches, which is the self-check `low_dram_report` runs.
fn log_with_disk_index(
    m: &Metrics,
    ac: &CacheProfile,
    dram: u64,
    di: DiskIndex,
    bound: Bound,
) -> Option<AtBudget> {
    let c = ac
        .with_index_entry_bytes(di.resident_bytes)
        .with_entry_bytes(di.cache_entry_bytes)
        .with_fixed_extra(di.fixed_bytes);
    // A new branch that splits an untouched leaf needs that leaf's record.
    let displaced = Extra {
        reads: ac.displaced_leaf_splits,
        rand_writes: 0,
    };
    let mut x = at_budget(m, &c, dram, bound, displaced)?;
    // Lookups a DRAM index answers for free: a branch read back after a
    // cache miss, a value read, a displaced leaf, and finding the old copy
    // of an overwritten or deleted record to mark it dead.
    let value_reads = m.page_reads - ac.accesses;
    let misses = if bound == Bound::Lo {
        x.reads_lo
    } else {
        x.reads_hi
    };
    let lookups = misses + value_reads + ac.displaced_leaf_splits + ac.liveness_lookups;
    x.m.page_reads += lookups * di.reads_per_lookup;
    x.m.read_bytes += lookups * di.reads_per_lookup * pbt_store::PAGE;
    let updates = m.record_writes + m.record_deletes + ac.relocations;
    let bytes = (updates as f64 * di.bytes_per_update) as u64;
    x.m.seq_bytes_written += bytes;
    x.m.seq_bytes_read += bytes;
    Some(x)
}

/// A cost cell: `(lo, hi)` per block, or None when the store cannot run.
type Span = Option<(f64, f64)>;

/// An LSM of records at report time: the engine constants above over the
/// replay's own update stream.
#[derive(Clone, Copy)]
struct LsmRecords {
    /// Bytes per entry: the mean record of the write stream, its 4-byte
    /// log header standing in for the varint lengths, plus the overhead.
    entry_bytes: f64,
    /// Entries written per record update over all levels, MEASURED by
    /// `lsm::cost_from_curve` on the replay's own update stream.
    amp: f64,
}

/// An LSM of records at a DRAM budget. `curve` already carries the
/// engine's fixed DRAM and the size of a cached entry; `updates` is the
/// records written plus deleted in the window, with no relocations:
/// compaction is the amplification. Reads: one random 4 KB block per
/// cache miss (ASSUMED: filters and index blocks are resident, so a lookup
/// probes one data block). Writes: every update goes to the WAL once and
/// is written `amp` times by flushes and compaction, which reads back what
/// it merges. One sync per block. `extra_reads` are lookups outside the
/// curve that every store pays, the displaced leaves whose key a new branch
/// needs. The log store's own metrics are not the base here: its
/// sequential bytes and record counts are its own.
fn lsm_of_records(
    updates: u64,
    extra_reads: u64,
    curve: &CacheProfile,
    dram: u64,
    cfg: LsmRecords,
    bound: Bound,
    blocks: u64,
) -> Option<AtBudget> {
    let cap = curve.capacity_for(dram)?;
    let (lo, hi) = curve.bounds_at(cap);
    let misses = if bound == Bound::Lo { lo } else { hi };
    let bytes = updates as f64 * cfg.entry_bytes;
    let m = Metrics {
        seq_bytes_written: (bytes * (cfg.amp + 1.0)) as u64,
        seq_bytes_read: (bytes * cfg.amp) as u64,
        page_reads: misses + extra_reads,
        read_bytes: (misses + extra_reads) * pbt_store::PAGE,
        fsyncs: blocks,
        ..Metrics::default()
    };
    Some(AtBudget {
        cap,
        reads_lo: lo,
        reads_hi: hi,
        m,
    })
}

/// Pages appended to a log at a DRAM budget: the page store's dirtied
/// pages written once each in sequence, found through an index in DRAM,
/// the log compacted when a fraction `g` of it is garbage (each garbage
/// page read 1/g times and rewritten (1-g)/g times, amortized). Every
/// appended page supersedes one, so the garbage equals the pages
/// appended; relocations update the DRAM index for free.
fn page_log(
    appended: u64,
    curve: &CacheProfile,
    dram: u64,
    g: f64,
    bound: Bound,
    blocks: u64,
) -> Option<AtBudget> {
    let cap = curve.capacity_for(dram)?;
    let (lo, hi) = curve.bounds_at(cap);
    let misses = if bound == Bound::Lo { lo } else { hi };
    let bytes = appended as f64 * pbt_store::PAGE as f64;
    let m = Metrics {
        seq_bytes_written: (bytes * (1.0 + (1.0 - g) / g)).round() as u64,
        seq_bytes_read: (bytes / g).round() as u64,
        page_reads: misses,
        read_bytes: misses * pbt_store::PAGE,
        fsyncs: PAGE_LOG_FSYNCS * blocks,
        ..Metrics::default()
    };
    Some(AtBudget {
        cap,
        reads_lo: lo,
        reads_hi: hi,
        m,
    })
}

/// What the record log's window looks like under its own compaction at
/// garbage fraction 1/2, from which other fractions are derived.
struct GcBase<'a> {
    arm: &'a Metrics,
    ac_true: &'a CacheProfile,
    /// Live records relocated in the window at g = 1/2: one per superseded.
    relocations_at_half: u64,
    /// Bytes the commits appended in the window.
    appended: u64,
    /// Garbage bytes the window created, compacted away at g = 1/2.
    garbage: u64,
    stream_updates: u64,
    index_curve: Option<&'a (Vec<(u64, f64)>, u64)>,
    records: u64,
    buffer: u64,
    assumed_amp: f64,
}

/// The record log store compacting when a fraction `g` of the log is
/// garbage instead of 1/2: per garbage byte it reads 1/g and rewrites
/// (1-g)/g, and relocates (1-g)/g live records per superseded one, each a
/// further index update. The index engine's amplification is recomputed
/// with that relocation share. Space is up to 1/(1-g) of the live data.
fn log_at_gc(base: &GcBase, g: f64) -> (Metrics, CacheProfile, f64) {
    let m = Metrics {
        seq_bytes_written: base.appended + (base.garbage as f64 * (1.0 - g) / g).round() as u64,
        seq_bytes_read: (base.garbage as f64 / g).round() as u64,
        ..base.arm.clone()
    };
    let mut c = base.ac_true.clone();
    c.relocations = (base.relocations_at_half as f64 * (1.0 - g) / g).round() as u64;
    let share = c.relocations as f64 / base.stream_updates.max(1) as f64;
    let amp = base.index_curve.map_or(base.assumed_amp, |(curve, len)| {
        index_amp_measured(
            pbt_store::lsm::cost_from_curve(
                curve,
                *len,
                share,
                pbt_store::lsm::LsmConfig {
                    entries: base.records,
                    entry_bytes: INDEX_ENTRY_DISK_BYTES,
                    buffer_bytes: base.buffer,
                    ratio: INDEX_SIZE_RATIO,
                },
            )
            .amplification,
            share,
        )
    });
    (m, c, amp)
}

/// "page, NOMT value store" -> "page": the family is the text before the
/// first comma.
fn family(name: &str) -> &str {
    name.split(',').next().unwrap_or(name).trim()
}

/// The cheapest family of stores. Each family stands by the variant with
/// the best worst case (`safest`, the smallest `hi`) and, where its best
/// case matters, by the variant with the smallest `lo` (`keenest`). W is
/// the family holding the smallest `hi` of all; R is the other family
/// nearest it by `lo`. Each store's unknown reads are its own, so ranges
/// are independent: the least W wins by is its `hi` against R's `lo`.
fn verdict(cells: &[Span], names: &[String]) -> String {
    struct Fam<'a> {
        name: &'a str,
        safest: (f64, f64),
        keenest: (f64, f64),
    }
    let mut fams: Vec<Fam> = Vec::new();
    for (span, name) in cells.iter().zip(names) {
        let Some(s) = *span else { continue };
        let f = family(name);
        match fams.iter_mut().find(|x| x.name == f) {
            Some(x) => {
                if s.1 < x.safest.1 {
                    x.safest = s;
                }
                if s.0 < x.keenest.0 {
                    x.keenest = s;
                }
            }
            None => fams.push(Fam {
                name: f,
                safest: s,
                keenest: s,
            }),
        }
    }
    match fams.len() {
        0 => return "-".to_string(),
        1 => return format!("{} (others cannot run)", fams[0].name),
        _ => {}
    }
    let w = (0..fams.len())
        .min_by(|a, b| fams[*a].safest.1.partial_cmp(&fams[*b].safest.1).unwrap())
        .unwrap();
    let r = (0..fams.len())
        .filter(|i| *i != w)
        .min_by(|a, b| fams[*a].keenest.0.partial_cmp(&fams[*b].keenest.0).unwrap())
        .unwrap();
    let (wf, rf) = (&fams[w], &fams[r]);
    let r_min = rf.keenest.0 / wf.safest.1;
    let r_max = rf.safest.1 / wf.safest.0;
    if r_min >= TOO_CLOSE {
        format!("{} ({r_min:.2}x to {r_max:.2}x over {})", wf.name, rf.name)
    } else if wf.safest.1 <= rf.keenest.0 {
        format!(
            "too close ({} by {r_min:.2}x to {r_max:.2}x over {})",
            wf.name, rf.name
        )
    } else {
        // The ranges overlap. If neither family could be ahead by the
        // threshold even at its best, the cell is close rather than open.
        let a = rf.safest.1 / wf.keenest.0;
        let b = wf.safest.1 / rf.keenest.0;
        let word = if a.max(b) < TOO_CLOSE {
            "too close"
        } else {
            "undecided"
        };
        format!(
            "{word} ({} by up to {a:.2}x, {} by up to {b:.2}x)",
            wf.name, rf.name
        )
    }
}

/// The levels of a modeled merge tree, as a table.
fn lsm_level_table(md: &mut String, c: &pbt_store::lsm::LsmCost, per_block: f64) {
    let _ = writeln!(md, "\n  | level | entries | fills in (blocks) | inside the replay | moves down per update | written per update |");
    let _ = writeln!(md, "  |---|---|---|---|---|---|");
    for (i, l) in c.levels.iter().enumerate() {
        let _ = writeln!(
            md,
            "  | {} | {:.3e} | {} | {} | {:.3} | {:.2} |",
            if i == 0 {
                "write buffer".to_string()
            } else {
                format!("L{i}")
            },
            l.capacity,
            l.window
                .map_or("-".to_string(), |w| format!("{:.0}", w / per_block)),
            if l.window.is_none() {
                "-"
            } else if l.measured {
                "yes"
            } else {
                "no: longest window's share used"
            },
            l.outflow,
            l.written,
        );
    }
    let _ = writeln!(md);
}

/// Below this ratio the "cheapest" column declines to name a winner. The
/// independent review of this model found biases of a few percent each, in
/// both directions, that together move a cell by up to about 1.3x.
const TOO_CLOSE: f64 = 1.3;

/// The comparison at the low end of DRAM, which is where a design has to
/// win if DRAM is a ceiling rather than something to buy more of.
fn low_dram_report(rows: &[Row], facts: &RunFacts) -> String {
    let mut md = String::new();
    let (Some((ar, ac)), Some((pr, pc))) = (
        profiled(rows, "append-readback"),
        profiled(rows, "paged-pos"),
    ) else {
        return md;
    };
    let Some(pvs) = pr.cache_vs.as_ref() else {
        return md;
    };
    let _ = writeln!(md, "\n## Under a DRAM ceiling: the low end\n");
    if !facts.value_reads {
        let _ = writeln!(
            md,
            "Not produced for witness-walk reads: the value-store model here covers value lookups only.\n"
        );
        return md;
    }
    let blocks = facts.blocks;
    let b = blocks.max(1) as f64;
    let value_reads = ar.m.page_reads - ac.accesses;
    let updates = ar.m.record_writes + ar.m.record_deletes + ac.relocations;

    // DRAM a commit holds while it runs, on top of everything cached: the
    // pages or records it is about to write. Mean per block, MEASURED.
    let ws_page_base = (pr.m.rand_page_writes as f64 / b * pbt_store::PAGE as f64) as u64;
    let ws_page_vs =
        ((pr.m.rand_page_writes + pvs.value_leaf_page_writes + pvs.value_bbn_writes) as f64 / b
            * pbt_store::PAGE as f64) as u64;
    let ws_log = (ar.m.seq_bytes_written as f64 / b) as u64;
    let pc_ws = pc.with_fixed_extra(ws_page_base);
    let pvs_ws = pvs.with_fixed_extra(ws_page_vs);
    let ac_ws = ac.with_fixed_extra(ws_log);

    // ASSUMED index engines. 16 B per entry on disk, write amplification 10.
    let filter = DiskIndex {
        resident_bytes: 1.5,
        cache_entry_bytes: 94,
        reads_per_lookup: 1,
        bytes_per_update: 160.0,
        fixed_bytes: 0,
    };
    let fingerprint = DiskIndex {
        resident_bytes: 4.25,
        cache_entry_bytes: 94,
        reads_per_lookup: 1,
        bytes_per_update: 160.0,
        fixed_bytes: 0,
    };
    // Self-check: a disk index that keeps everything in DRAM and charges
    // nothing must reproduce the DRAM-index column exactly.
    let in_dram = DiskIndex {
        resident_bytes: ac.index_entry_bytes,
        cache_entry_bytes: ac.entry_bytes,
        reads_per_lookup: 0,
        bytes_per_update: 0.0,
        fixed_bytes: 0,
    };
    let displaced = Extra {
        reads: ac.displaced_leaf_splits,
        rand_writes: 0,
    };

    let _ = writeln!(
        md,
        "> Nature: SYNTHETIC workload, COUNTER model. The base model cannot run the\n> log store below its index, and it charges the page store nothing for\n> persisting leaf keys and values. This section fills both gaps, and the two\n> sides are NOT filled to the same standard. The page side is NOMT as built:\n> mechanism read from its source, counts measured on a modeled value store.\n> The log side with its index on disk is an ASSUMED engine, because none is\n> implemented here; read those columns as what such an engine would have to\n> achieve. The log side with its index in DRAM keeps that index volatile:\n> persisting and recovering it is charged nowhere.\n"
    );
    let _ = writeln!(md, "- workload: {}", facts.workload);
    let _ = writeln!(
        md,
        "- measured, log store, per block: {:.1} index updates ({:.1} records written, {:.1} deleted, {:.1} relocated by compaction); {:.1} lookups to find the old copy of an overwritten or deleted record; {:.1} leaves displaced by a new branch; {:.1} value reads; {:.2} MB appended",
        updates as f64 / b,
        ar.m.record_writes as f64 / b,
        ar.m.record_deletes as f64 / b,
        ac.relocations as f64 / b,
        ac.liveness_lookups as f64 / b,
        ac.displaced_leaf_splits as f64 / b,
        value_reads as f64 / b,
        ws_log as f64 / MB,
    );
    let _ = writeln!(
        md,
        "- measured, page store, per block: {:.1} changed leaves, which dirty {:.1} distinct value-store leaf pages out of {} and {:.1} distinct bottom branch nodes out of {}; {:.1} merkle pages written",
        pc.uncharged_leaf_changes as f64 / b,
        pvs.value_leaf_page_writes as f64 / b,
        pvs.value_leaf_pages,
        pvs.value_bbn_writes as f64 / b,
        pvs.value_bbns,
        pr.m.rand_page_writes as f64 / b,
    );
    let _ = writeln!(
        md,
        "- every column also holds, as fixed DRAM, what one commit is about to write (mean per block): {:.2} MB for the page store as modeled, {:.2} MB with its value store, {:.2} MB for the log store.",
        ws_page_base as f64 / MB,
        ws_page_vs as f64 / MB,
        ws_log as f64 / MB,
    );
    let _ = writeln!(
        md,
        "- `page, as modeled`: the base model. Merkle pages only, three fsyncs, leaf keys and values persisted nowhere. Shown for reference; it is not a store that could run."
    );
    let _ = writeln!(
        md,
        "- `page, NOMT value store`: leaf pages of at most {} entries share the cache with merkle pages; each dirtied leaf page is read if uncached and rewritten copy-on-write every commit, as is the bottom branch node above it (at most {} leaf pages per node, ASSUMED); every such node is resident; four free-list page writes per commit (ASSUMED); five fsyncs. This is specific to NOMT's value store. A page store that logged or batched its values would cost less.",
        pbt_store::paged::VALUE_LEAF_ENTRIES,
        pbt_store::paged::VALUE_BBN_MAX,
    );
    let _ = writeln!(
        md,
        "- `log, index in DRAM`: the base model at {} B per index entry, plus one record read per displaced leaf. Cannot run below its index.",
        ac.index_entry_bytes
    );
    let _ = writeln!(
        md,
        "- `log, index on disk` (ASSUMED): {} or {} B per index entry stays in DRAM; a cached branch costs {} B; every lookup the DRAM part cannot answer costs {} random 4 KB read on top of the record read; every index update writes and reads {} B sequentially (16 B entry, write amplification 10). Inside these figures the lookups that find a superseded copy cost {:.0} per block on `{}` and {:.0} on `{}`; an engine that learns that some other way does not pay them.",
        filter.resident_bytes,
        fingerprint.resident_bytes,
        filter.cache_entry_bytes,
        filter.reads_per_lookup,
        filter.bytes_per_update,
        ac.liveness_lookups as f64 / b * filter.reads_per_lookup as f64 * PROFILES[0].c_rand_read,
        PROFILES[0].name,
        ac.liveness_lookups as f64 / b * filter.reads_per_lookup as f64 * PROFILES[1].c_rand_read,
        PROFILES[1].name,
    );
    let _ = writeln!(
        md,
        "- budgets are per state key ({} live leaves); per log record, branches included, they are half that. This is a unit for reading across state sizes, not a claim that cost scales with it.",
        ac.state_keys
    );
    let _ = writeln!(
        md,
        "- `cheapest` compares the page store with NOMT's value store against the best log column. Under {TOO_CLOSE}x it names no winner: known biases of a few percent each, in both directions, are enough to flip such a cell. The page-elision rule here counts a page's own slots where NOMT counts leaves under it, which alone is worth 2-5% in the page store's favour.\n"
    );

    let per_key = [1.0f64, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0];
    for prof in PROFILES.iter() {
        for (price, label) in [(Price::PerIo, "per IO"), (Price::Bandwidth, "by bytes")] {
            let _ = writeln!(
                md,
                "### IO cost per block under a ceiling, profile `{}`, reads priced {}\n",
                prof.name, label
            );
            let _ = writeln!(
                md,
                "| DRAM B per state key | DRAM MB | page, as modeled | page, NOMT value store | log, index in DRAM | log, index on disk ({} B) | log, index on disk ({} B) | cheapest |",
                filter.resident_bytes, fingerprint.resident_bytes
            );
            let _ = writeln!(md, "|---|---|---|---|---|---|---|---|");
            for &bpk in &per_key {
                let d = (bpk * ac.state_keys as f64) as u64;
                let cost = |x: Option<AtBudget>| x.map(|x| io_cost(&x.m, prof, blocks, price));
                let page = cost(at_budget(&pr.m, &pc_ws, d, Bound::Hi, Extra::default()));
                let page_vs = cost(page_with_value_store(
                    &pr.m,
                    pc,
                    &pvs_ws,
                    d,
                    blocks,
                    Bound::Hi,
                ));
                let log_ram = cost(at_budget(&ar.m, &ac_ws, d, Bound::Hi, displaced));
                let check = cost(log_with_disk_index(&ar.m, &ac_ws, d, in_dram, Bound::Hi));
                assert!(
                    match (log_ram, check) {
                        (Some(x), Some(y)) => (x - y).abs() < 1e-6 * x.max(1.0),
                        (None, None) => true,
                        _ => false,
                    },
                    "disk-index model does not reduce to the DRAM-index model"
                );
                let log_f = cost(log_with_disk_index(&ar.m, &ac_ws, d, filter, Bound::Hi));
                let log_g = cost(log_with_disk_index(
                    &ar.m,
                    &ac_ws,
                    d,
                    fingerprint,
                    Bound::Hi,
                ));
                if page.is_none()
                    && page_vs.is_none()
                    && log_ram.is_none()
                    && log_f.is_none()
                    && log_g.is_none()
                {
                    continue; // nothing fits this budget
                }
                let fmt = |x: Option<f64>| x.map_or("-".to_string(), |v| format!("{v:.0}"));
                let best_log = [
                    ("log, index in DRAM".to_string(), log_ram),
                    (
                        format!("log, index on disk ({} B)", filter.resident_bytes),
                        log_f,
                    ),
                    (
                        format!("log, index on disk ({} B)", fingerprint.resident_bytes),
                        log_g,
                    ),
                ]
                .into_iter()
                .filter_map(|(n, c)| c.map(|c| (n, c)))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                let verdict = match (page_vs, best_log) {
                    (Some(p), Some((n, l))) => {
                        let (winner, ratio) = if l < p {
                            (n, p / l)
                        } else {
                            ("page".to_string(), l / p)
                        };
                        if ratio < TOO_CLOSE {
                            format!("too close ({winner} by {ratio:.2}x)")
                        } else {
                            format!("{winner} ({ratio:.2}x)")
                        }
                    }
                    (Some(_), None) => "page (log cannot run)".to_string(),
                    (None, Some((n, _))) => format!("{n} (page cannot run)"),
                    (None, None) => "-".to_string(),
                };
                let _ = writeln!(
                    md,
                    "| {} | {:.1} | {} | {} | {} | {} | {} | {} |",
                    bpk,
                    d as f64 / MB,
                    fmt(page),
                    fmt(page_vs),
                    fmt(log_ram),
                    fmt(log_f),
                    fmt(log_g),
                    verdict,
                );
            }
            let _ = writeln!(md);
        }
    }
    md
}

const GB: f64 = (1u64 << 30) as f64;

/// ASSUMED index engine for a log store whose index is on disk: a leveled
/// merge tree of 16-byte entries with a 256 MB write buffer and a size
/// ratio of 10. Its write amplification follows from how many levels the
/// full state's index needs, at half the size ratio per level.
const INDEX_ENTRY_DISK_BYTES: f64 = 16.0;
const INDEX_BUFFER_BYTES: u64 = 256 << 20;
const INDEX_SIZE_RATIO: f64 = 10.0;

/// ASSUMED existing-engine baseline: every PBT record is one entry of a
/// leveled merge tree keyed by tree position, the layout a geth (Pebble)
/// or Nethermind (RocksDB) node keeps its trie in, with the index engine's
/// buffer and size ratio. Per entry, 8 B of sequence number and type on
/// top of the record (prefix compression is not modeled); 1.85 B resident,
/// a 10-bit bloom filter (1.25 B) and the index blocks (about 0.6 B); and
/// 16 B of recency bookkeeping per cached record, as the log store's cache
/// is charged.
const LSM_RECORD_OVERHEAD_BYTES: f64 = 8.0;
const LSM_RESIDENT_BYTES_PER_ENTRY: f64 = 1.85;
const LSM_CACHE_OVERHEAD_BYTES: u64 = 16;
/// What a real engine wrote against the LSM-of-records model, MEASURED
/// (2026-10-27) on RocksDB 11.8.1 fed part 0's record stream of the 24.0M
/// window (`results/lsm_validate_24000000_p0_summary.md`, run
/// `geth64b4i5`): the part's memtable (64 MB) and level base (2 MB, so the
/// tree has the five levels the part's state gives geth's base), path keys,
/// snappy, cold ballast interleaved with the stream's keys standing in for
/// the untouched state, 15,000 blocks at steady state. Entries written by
/// flushes and compactions per update (the engine's event log) against the
/// model's on the same stream; the four interleaved runs gave 0.96 to 1.11.
/// The model's amplification is scaled by this. The engine's bytes per
/// entry written were 64 B against the model's 81 B (prefix compression and
/// snappy, not modeled), which the tables keep as the model's.
const LSM_RECORDS_ENGINE_OVER_MODEL: f64 = 1.01;
/// The disk index's engine, MEASURED the same way (runs `idx64b8i` and
/// `idx64b8g23i`, ballast 8 interleaved) with 16 B hashed keys, 8 B values
/// and the relocations as random puts: entries the engine wrote per update
/// over the model's, at half a relocation per update (g = 2/3) and at one
/// (g = 1/2). The model's amplification is scaled by the fraction, linear in
/// the relocation share between the two points and held outside them.
const INDEX_AMP_ENGINE_OVER_MODEL: [(f64, f64); 2] = [(0.5, 0.88), (1.0, 0.83)];

fn index_amp_measured(model_amp: f64, reloc_share: f64) -> f64 {
    let [(r0, f0), (r1, f1)] = INDEX_AMP_ENGINE_OVER_MODEL;
    let t = ((reloc_share - r0) / (r1 - r0)).clamp(0.0, 1.0);
    model_amp * (f0 + t * (f1 - f0))
}
/// ASSUMED page-granular stores: a page log keeps one 16 B entry per page
/// (id and log position) in DRAM and syncs twice per block (log, then
/// manifest); a page LSM holds a 4 KB page plus its 16 B key per entry.
const PAGE_INDEX_ENTRY_BYTES: u64 = 16;
const PAGE_LOG_FSYNCS: u64 = 2;
const PAGE_LSM_ENTRY_BYTES: f64 = 4112.0;

fn index_levels(records: u64, entry_bytes: f64, buffer: u64) -> f64 {
    (records as f64 * entry_bytes / buffer as f64)
        .log(INDEX_SIZE_RATIO)
        .ceil()
        .max(1.0)
}

/// The two stores under a full node's DRAM, with mainnet's write stream.
/// Returns the report section and the curves behind it as CSV.
///
/// The cache curves come from the replay. Everything that scales with the
/// size of the state (the index, the resident value-store nodes) is taken
/// from the full state the replay stands in for, not from the replay's own
/// record count, which is a sliver of it.
fn mainnet_report(rows: &[Row], facts: &RunFacts, mn: &MainnetCtx) -> (String, String) {
    let mut md = String::new();
    let (Some((ar, ac)), Some((pr, pc))) = (
        profiled(rows, "append-readback"),
        profiled(rows, "paged-pos"),
    ) else {
        return (md, String::new());
    };
    let Some(pvs) = pr.cache_vs.as_ref() else {
        return (md, String::new());
    };
    // A sampled replay covers one key-space subtree in k: everything that
    // scales with the state is taken at 1/k, DRAM budgets are applied at
    // 1/k and shown whole, and per-block figures divide by blocks / k.
    let k = 1u64 << mn.facts.sample_bits;
    assert_eq!(
        facts.blocks % k,
        0,
        "measured blocks must divide by the sample factor"
    );
    let blocks = facts.blocks / k;
    let b = blocks.max(1) as f64;
    let gbk = GB / k as f64;
    // fsyncs are per commit, not per key: undo the scaling for them.
    let arm = Metrics {
        fsyncs: ar.m.fsyncs / k,
        ..ar.m.clone()
    };
    let prm = Metrics {
        fsyncs: pr.m.fsyncs / k,
        ..pr.m.clone()
    };
    let buffer = INDEX_BUFFER_BYTES / k;
    let sc = &mn.scale;
    let f = &mn.facts;
    let leaves = sc.leaves() / k;
    let records = 2 * leaves;

    let ws_page =
        ((prm.rand_page_writes + pvs.value_leaf_page_writes + pvs.value_bbn_writes) as f64 / b
            * pbt_store::PAGE as f64) as u64;
    let ws_log = (ac.appended_bytes as f64 / b) as u64;
    // Value-store geometry of the full state: leaf pages at typical fill,
    // one resident bottom branch node per ~357 of them. NOMT also keeps one
    // byte per hash-table bucket in memory; ASSUMED here at half load with
    // as many merkle pages as value-store leaf pages.
    let leaf_pages = sc.value_leaf_pages() / k;
    let bbn_bytes = (leaf_pages as f64 / mainnet::VALUE_BBN_FILL) as u64 * pbt_store::PAGE;
    let bucket_map = 2 * leaf_pages;
    // Leaves share the log store's cache with branches, so every record is
    // cacheable.
    let ac_true = ac
        .with_index_entries(records)
        .with_cacheable(records)
        .with_fixed_extra(ws_log);
    let pvs_true = pvs
        .with_cacheable(2 * leaf_pages)
        .with_fixed_extra(bbn_bytes + bucket_map + ws_page);

    let levels = index_levels(records, INDEX_ENTRY_DISK_BYTES, buffer);
    let assumed_amp = levels * INDEX_SIZE_RATIO / 2.0;
    // The engine laid out over the full index, fed the replay's own update
    // stream (warm-up included): what it absorbs is measured, the layout is
    // the standard one.
    let stream_updates = arm.record_writes + arm.record_deletes;
    let lsm_cost = ar.index_curve.as_ref().map(|(curve, len)| {
        pbt_store::lsm::cost_from_curve(
            curve,
            *len,
            ac.relocations as f64 / stream_updates.max(1) as f64,
            pbt_store::lsm::LsmConfig {
                entries: records,
                entry_bytes: INDEX_ENTRY_DISK_BYTES,
                buffer_bytes: buffer,
                ratio: INDEX_SIZE_RATIO,
            },
        )
    });
    let amp_model = lsm_cost.as_ref().map_or(assumed_amp, |c| c.amplification);
    // The tables use what a real engine wrote; the model stays in the prose.
    let amp = lsm_cost.as_ref().map_or(assumed_amp, |c| {
        index_amp_measured(c.amplification, c.relocation_share)
    });
    let index_disk = records as f64 * INDEX_ENTRY_DISK_BYTES;
    // One 16-byte pointer per 4 KB index page, so a lookup is one read.
    let index_fixed = buffer + (index_disk / pbt_store::PAGE as f64 * 16.0) as u64;
    let on_disk = |resident_bytes: f64, amp: f64| DiskIndex {
        resident_bytes,
        // A cached record also remembers where its logged copy is.
        cache_entry_bytes: ac.entry_bytes + 8,
        reads_per_lookup: 1,
        bytes_per_update: INDEX_ENTRY_DISK_BYTES * amp,
        fixed_bytes: index_fixed,
    };
    let (filter, fingerprint) = (on_disk(1.5, amp), on_disk(4.25, amp));
    let displaced = Extra {
        reads: ac.displaced_leaf_splits,
        rand_writes: 0,
    };
    let updates = arm.record_writes + arm.record_deletes + ac.relocations;
    let fixed_of = |di: DiskIndex| {
        ac_true
            .with_index_entry_bytes(di.resident_bytes)
            .fixed_dram_bytes
            + di.fixed_bytes
    };
    // Existing-engine baselines, MODELED from the same replay. An LSM of
    // records: the write stream's mean record is the entry, fed through the
    // same engine as the index, with no relocations.
    let mean_record = ac.appended_bytes as f64 / arm.record_writes.max(1) as f64;
    let lsm_updates = arm.record_writes + arm.record_deletes;
    let lsm_entry_bytes = mean_record + LSM_RECORD_OVERHEAD_BYTES;
    let lsm_records_cost = ar.index_curve.as_ref().map(|(curve, len)| {
        pbt_store::lsm::cost_from_curve(
            curve,
            *len,
            0.0,
            pbt_store::lsm::LsmConfig {
                entries: records,
                entry_bytes: lsm_entry_bytes,
                buffer_bytes: buffer,
                ratio: INDEX_SIZE_RATIO,
            },
        )
    });
    let lsm_amp_model = lsm_records_cost.as_ref().map_or(
        index_levels(records, lsm_entry_bytes, buffer) * INDEX_SIZE_RATIO / 2.0,
        |c| c.amplification,
    );
    // The tables use the model scaled by what a real engine wrote; the
    // model's own figure stays in the prose and the sensitivity table.
    let lsm_amp = if lsm_records_cost.is_some() {
        lsm_amp_model * LSM_RECORDS_ENGINE_OVER_MODEL
    } else {
        lsm_amp_model
    };
    let lr = LsmRecords {
        entry_bytes: lsm_entry_bytes,
        amp: lsm_amp,
    };
    // Node cache: the log store's record-granular stream, with the LSM's
    // resident part in place of the log's index. The log's resident top is
    // already in the curve's fixed DRAM; the buffer is the memtable the
    // commit's records sit in.
    let lsm_node = ac
        .with_index(records, LSM_RESIDENT_BYTES_PER_ENTRY)
        .with_cacheable(records)
        .with_fixed_extra(buffer)
        .with_entry_bytes(lsm_entry_bytes.round() as u64 + LSM_CACHE_OVERHEAD_BYTES);
    // The B+tree of records, if it ran: its own curve over leaf and
    // interior pages for its column, its leaf-page curve as the LSM's block
    // cache. Its page count is the full state's; take it at 1/k.
    let bt = profiled(rows, "btree");
    let bt_true = bt.map(|(br, bc)| {
        let btm = Metrics {
            fsyncs: br.m.fsyncs / k,
            ..br.m.clone()
        };
        let ws_bt = (btm.rand_page_writes as f64 / b * pbt_store::PAGE as f64) as u64;
        let c = bc
            .with_cacheable(bc.cacheable_entries / k)
            .with_fixed_extra(ws_bt);
        (btm, c)
    });
    let bt_leaf = bt.and_then(|(br, _)| br.cache_vs.as_ref());
    let lsm_block = bt_leaf.map(|lc| {
        lc.with_index(records, LSM_RESIDENT_BYTES_PER_ENTRY)
            .with_cacheable(lc.cacheable_entries / k)
            .with_fixed_extra(buffer)
    });
    // The page row of the design space, from the page store's own run: its
    // dirtied pages appended to a log or put into a merge tree, and its
    // page cache standing in for the block cache of an LSM of records
    // whose key order makes a block one page.
    let page_index_bytes = 2 * leaf_pages * PAGE_INDEX_ENTRY_BYTES;
    let pages_appended = prm.rand_page_writes + pvs.value_leaf_page_writes;
    let pl_curve = pvs
        .with_cacheable(2 * leaf_pages)
        .with_fixed_extra(page_index_bytes + ws_page);
    let page_lsm_cost = pr
        .index_curve
        .as_ref()
        .filter(|(c, len)| *len > 0 && c.last().is_some_and(|x| x.1 > 0.0))
        .map(|(c, len)| {
            pbt_store::lsm::cost_from_curve(
                c,
                *len,
                0.0,
                pbt_store::lsm::LsmConfig {
                    entries: 2 * leaf_pages,
                    entry_bytes: PAGE_LSM_ENTRY_BYTES,
                    buffer_bytes: buffer,
                    ratio: INDEX_SIZE_RATIO,
                },
            )
        });
    let page_lsm = page_lsm_cost.as_ref().map(|c| LsmRecords {
        entry_bytes: PAGE_LSM_ENTRY_BYTES,
        amp: c.amplification,
    });
    let page_lsm_curve = pvs
        .with_cacheable(2 * leaf_pages)
        .with_index(2 * leaf_pages, LSM_RESIDENT_BYTES_PER_ENTRY)
        .with_fixed_extra(buffer);
    let lsm_paged = pvs
        .with_cacheable(2 * leaf_pages)
        .with_index(records, LSM_RESIDENT_BYTES_PER_ENTRY)
        .with_fixed_extra(buffer);
    // An elided page's records sit in the block the changed key's own read
    // fetches (the page store reads that key's value page; here the leaf
    // record is in its bottom page's block), so they add no read of their own.
    let lsm_paged_extra = ac.displaced_leaf_splits;
    // The record log with lazier garbage collection.
    let gc_base = GcBase {
        arm: &arm,
        ac_true: &ac_true,
        relocations_at_half: ac.relocations,
        appended: ac.appended_bytes,
        garbage: arm.seq_bytes_written.saturating_sub(ac.appended_bytes),
        stream_updates,
        index_curve: ar.index_curve.as_ref(),
        records,
        buffer,
        assumed_amp,
    };
    let gc23 = log_at_gc(&gc_base, 2.0 / 3.0);
    let gc23_di = on_disk(1.5, gc23.2);

    let _ = writeln!(md, "\n## Mainnet replay under a full node's DRAM\n");
    let _ = writeln!(
        md,
        "> Nature: MAINNET write stream, replayed on a tree whose untouched state is\n> MODELED, costed with the COUNTER model. Which keys each block writes, and\n> whether each write creates, rewrites or deletes, is real. The rest of the\n> state is not loaded: beside every written path, wherever the full state has\n> keys on the other side of a branch, a stand-in is inserted, so paths have\n> the depth they would have on mainnet. Where the full state has keys is\n> taken from its size and EIP-8297's hash placement, not from the keys\n> themselves. The page side is NOMT as built; the log side with its index on\n> disk is an ASSUMED engine. Two existing-engine baselines, an LSM of records\n> and a copy-on-write B+tree, are MODELED from the same replay. {}\n",
        if facts.exec_reads == (0, 0) {
            "Only writes are replayed: no execution reads were given, and code and\n> delegations are left out."
        } else {
            "Execution reads of balances, nonces and storage are replayed from the\n> same dataset; code reads and delegations are left out."
        }
    );
    let _ = writeln!(md, "- workload: {}", facts.workload);
    if k > 1 {
        let _ = writeln!(
            md,
            "- SAMPLED: only accounts and contracts whose address hash starts with the {} bits of part {} are replayed, one key-space subtree in {}. Every figure below is scaled to the whole: per-block counts by {}, cache sizes and DRAM by {}, the full state and the index buffer taken at 1/{} for the run. Counts in the next bullet are of the sample. Commits are not sampled: fsyncs are not scaled.",
            mn.facts.sample_bits, mn.facts.sample_part, k, k, k, k
        );
    }
    let _ = writeln!(
        md,
        "- window: {} measured blocks after {} warm-up blocks; {} blocks available from {}",
        facts.blocks, facts.warmup, f.blocks, f.first_block
    );
    let _ = writeln!(
        md,
        "- full state stood in for: {} accounts, {} storage slots, {} code chunks = {} leaves, {} records. Storage contracts: {} (ASSUMED). An untouched contract is taken to hold {} slots{}.",
        sc.accounts,
        sc.storage_slots,
        sc.code_chunks(),
        leaves,
        records,
        sc.storage_contracts,
        sc.neighbor_slots(),
        if sc.neighbor_slots.is_some() { " (set for this run)" } else { ", the mean; real sizes are heavy-tailed" },
    );
    let _ = writeln!(
        md,
        "- replay built from: {} account changes ({} creations) and {} slot changes; {} keys in the starting state; stand-ins: {} beside account paths, {} beside contract buckets, {} inside contracts' storage. {} of {} contracts written in the storage zone have a fetched slot count, covering {:.1}% of the changes there; the rest are taken to have as many untouched stems as written ones. The sample writes {} distinct slots there in {} stems ({:.2} per stem), and each contract's untouched slots are taken to sit as many to a stem as its written ones.",
        f.account_changes,
        f.account_creations,
        f.slot_changes,
        f.genesis_keys,
        f.stand_ins_accounts,
        f.stand_ins_contracts,
        f.stand_ins_groups,
        f.contracts_with_count,
        f.contracts,
        100.0 * f.slot_changes_with_count,
        f.written_slots,
        f.written_stems,
        f.written_slots as f64 / f.written_stems.max(1) as f64,
    );
    for (zone, (n, sum, max)) in &facts.leaf_depths.zones {
        let _ = writeln!(
            md,
            "- path length, {} zone: {:.1} leaves written per block, {:.1} branch records above each on average, {} at most",
            zone,
            *n as f64 / b,
            *sum as f64 / (*n).max(1) as f64,
            max
        );
    }
    let (exec_present, exec_absent) = facts.exec_reads;
    if exec_present + exec_absent > 0 {
        let _ = writeln!(
            md,
            "- execution reads, per block: {:.1} of existing keys and {:.1} of absent ones, each key the block reads but does not change, once per block. They are served before the block's commit, through the same cache as the write path. The log store fetches the leaf record of an existing key and answers an absent one from its index, or from the per-entry filter of an index on disk, without IO. The page store reads the value-store leaf page where the key sits or would sit, either way.",
            exec_present as f64 / b,
            exec_absent as f64 / b,
        );
    }
    let _ = writeln!(
        md,
        "- measured, log store, per block: {:.1} records written, {:.2} MB appended by the commit; with no cache {:.1} reads, of which {:.1} are execution reads, {:.1} fetch the old copy of an overwritten or deleted leaf and the rest read back a branch; {:.1} index updates, {:.1} deleted branches to mark dead, {:.1} displaced leaves",
        arm.record_writes as f64 / b,
        ws_log as f64 / MB,
        ac.accesses as f64 / b,
        exec_present as f64 / b,
        ac.leaf_fetches.saturating_sub(exec_present) as f64 / b,
        updates as f64 / b,
        ac.liveness_lookups as f64 / b,
        ac.displaced_leaf_splits as f64 / b,
    );
    let _ = writeln!(
        md,
        "- measured, page store, per block: {:.1} merkle pages written; with no cache {:.1} reads, of which {:.1} are merkle pages and the rest value-store leaf pages (execution reads included); {:.1} value-store leaf pages and {:.1} bottom branch nodes rewritten, {:.2} MB held by one commit",
        prm.rand_page_writes as f64 / b,
        pvs.accesses as f64 / b,
        pc.accesses as f64 / b,
        pvs.value_leaf_page_writes as f64 / b,
        pvs.value_bbn_writes as f64 / b,
        ws_page as f64 / MB,
    );
    let _ = writeln!(
        md,
        "- both stores are charged the read of a value before it is overwritten or deleted. For the page store that is the value-store leaf page it rewrites. For the log store it is the old leaf record, kept in the same cache as branches at {} B an entry on average; a commit by itself would write the leaf blind, but the transaction that changes a value has read it.",
        ac.entry_bytes,
    );
    let _ = writeln!(
        md,
        "- fixed DRAM at full-state size: log store with its index in DRAM {:.1} GB ({} B per record); with its index on disk {:.1} GB or {:.1} GB ({} or {} B per record, plus a {} MB write buffer and {} MB of page pointers, all ASSUMED); page store {:.2} GB ({:.2} GB of resident bottom branch nodes over {} value-store leaf pages, {:.2} GB of bucket map, ASSUMED, and one commit's pages).",
        ac_true.fixed_dram_bytes as f64 / gbk,
        ac.index_entry_bytes,
        fixed_of(filter) as f64 / gbk,
        fixed_of(fingerprint) as f64 / gbk,
        filter.resident_bytes,
        fingerprint.resident_bytes,
        INDEX_BUFFER_BYTES >> 20,
        ((index_fixed - buffer) * k) >> 20,
        pvs_true.fixed_dram_bytes as f64 / gbk,
        bbn_bytes as f64 / gbk,
        leaf_pages * k,
        bucket_map as f64 / gbk,
    );
    let _ = writeln!(
        md,
        "- what the replay can and cannot resolve: a read of something untouched since the replay began is a miss for any cache smaller than everything touched before it, and an unknown for a larger one. Costs are given as `lo-hi`: `lo` if every unknown is a hit, `hi` if every one is a miss. The two agree up to what the warm-up touched, {:.2} GB of records for the log store and {:.2} GB of pages for the page store. A replay of {} blocks cannot tell further; a longer one narrows the range.",
        (ac.exact_upto * filter.cache_entry_bytes) as f64 / gbk,
        (pvs.exact_upto * pvs.entry_bytes) as f64 / gbk,
        f.blocks,
    );
    let upd_pages = updates as f64 / b * filter.bytes_per_update / pbt_store::PAGE as f64;
    if let Some(c) = &lsm_cost {
        let per_block = (c.stream_updates * k) as f64 / (facts.warmup + facts.blocks).max(1) as f64;
        let _ = writeln!(
            md,
            "- inside every index-on-disk figure, the index engine MODELED from this replay: a leveled merge tree over {:.0} GB of 16-byte entries, a {} MB write buffer, levels growing by {}x down to one that holds every entry. How much each level absorbs is MEASURED from the {} index updates of the replay ({:.0} per block, warm-up included): an entry updated again while it sits in the buffer or an upper level never moves further down. The {:.1} relocations per update are taken never to be absorbed. Write amplification {:.1} per index update ({:.0} B written and read): the model gives {:.1}, and a real RocksDB fed this stream as the index sees it wrote {:.2} of that (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); {:.0} for the assumed engine before. That costs {:.0} per block on `{}` and {:.0} on `{}`. Every lookup the cache cannot answer costs one more 4 KB read (ASSUMED, as before). A log whose branches are found without an index pays neither for branches.",
            index_disk / gbk,
            INDEX_BUFFER_BYTES >> 20,
            INDEX_SIZE_RATIO,
            c.stream_updates,
            per_block,
            c.relocation_share,
            amp,
            filter.bytes_per_update,
            amp_model,
            amp / amp_model,
            assumed_amp,
            upd_pages * (PROFILES[0].c_seq4k_write + PROFILES[0].c_seq4k_read),
            PROFILES[0].name,
            upd_pages * (PROFILES[1].c_seq4k_write + PROFILES[1].c_seq4k_read),
            PROFILES[1].name,
        );
        lsm_level_table(&mut md, c, per_block);
    } else {
        let _ = writeln!(
        md,
        "- inside every index-on-disk figure, the ASSUMED index engine: {:.0} GB of 16-byte entries in {} levels, so each of the {:.1} index updates per block is written and read {:.0} times over ({:.0} B). That costs {:.0} per block on `{}` and {:.0} on `{}`. Every lookup the cache cannot answer costs one more 4 KB read. A log whose branches are found without an index pays neither for branches.",
        index_disk / gbk,
        levels,
        updates as f64 / b,
        amp,
        filter.bytes_per_update,
        upd_pages * (PROFILES[0].c_seq4k_write + PROFILES[0].c_seq4k_read),
        PROFILES[0].name,
        upd_pages * (PROFILES[1].c_seq4k_write + PROFILES[1].c_seq4k_read),
        PROFILES[1].name,
    );
    }
    // The existing-engine baselines, laid out over the same full state.
    let lsm_bytes_per_update = lsm_entry_bytes * lsm_amp;
    let lsm_pages = lsm_updates as f64 / b * lsm_bytes_per_update / pbt_store::PAGE as f64;
    let _ = writeln!(
        md,
        "- existing-engine baseline, LSM of records, MODELED from this replay with its write amplification MEASURED on RocksDB: every record is one entry of a leveled merge tree keyed by tree position, the layout a geth (Pebble) or Nethermind (RocksDB) node keeps its trie in, with the engine of the index above: a {} MB write buffer and levels growing by {}x down to one holding all {} records. An entry is {:.0} B, the mean record of this replay's write stream ({:.1} B, its 4 B log header standing in for the varint lengths) plus {:.0} B of sequence number and type; prefix compression is not modeled. Updates are the {:.1} records written or deleted per block, with no relocations; the displaced leaves a new branch needs are one read each, as for the log store. Record write amplification {:.1} per record update ({:.0} B written and read back by compaction, plus one WAL copy of each update), the model's {:.1} scaled by {:.2}, what a real RocksDB fed this part's record stream through the engine above at the part's scale, five levels, cold ballast interleaved for the untouched state, wrote of it (MEASURED, `results/lsm_validate_24000000_p0_summary.md`); against {:.1} for the index entries above; that costs {:.0} per block on `{}` and {:.0} on `{}`. Fixed DRAM {:.2} GB: {} B per record of bloom filter (10 bits per key) and index blocks, and the write buffer, ASSUMED, plus the resident top. A lookup the cache cannot answer costs one random 4 KB read (ASSUMED: filters and index blocks resident, so one data block is probed). `LSM, node cache` caches single records at {} B each over the log store's record-granular stream; `LSM, block cache` caches 4 KB blocks over the B+tree's leaf-page stream, the same keys at block granularity. The first is pessimistic for such an engine, the second optimistic: a real one, a block cache under a clean node cache, sits between.",
        INDEX_BUFFER_BYTES >> 20,
        INDEX_SIZE_RATIO,
        records * k,
        lsm_entry_bytes,
        mean_record,
        LSM_RECORD_OVERHEAD_BYTES,
        lsm_updates as f64 / b,
        lsm_amp,
        lsm_bytes_per_update,
        lsm_amp_model,
        LSM_RECORDS_ENGINE_OVER_MODEL,
        amp,
        lsm_pages * (PROFILES[0].c_seq4k_write + PROFILES[0].c_seq4k_read),
        PROFILES[0].name,
        lsm_pages * (PROFILES[1].c_seq4k_write + PROFILES[1].c_seq4k_read),
        PROFILES[1].name,
        lsm_node.fixed_dram_bytes as f64 / gbk,
        LSM_RESIDENT_BYTES_PER_ENTRY,
        lsm_node.entry_bytes,
    );
    if let Some(c) = &lsm_records_cost {
        let per_block = (c.stream_updates * k) as f64 / (facts.warmup + facts.blocks).max(1) as f64;
        lsm_level_table(&mut md, c, per_block);
    }
    if let Some((btm, btc)) = &bt_true {
        use pbt_store::btree::{FANOUT, FILL, LEAF_ENTRIES, MEAN_ENTRY_BYTES};
        let g = pbt_store::btree::Geometry::for_records(records * k);
        let leaf_reads = bt_leaf.map_or(0, |c| c.accesses);
        let _ = writeln!(
            md,
            "- existing-engine baseline, copy-on-write B+tree, MODELED from this replay: records in key order in 4 KB pages, {} per leaf page ({} B mean entry at fill {}, ASSUMED), {} children per interior page (ASSUMED), height {} over {} leaf pages ({:.0} GB), the layout a reth node keeps its trie in (MDBX). A commit reads every page on the way to the ones it changes, copies each changed leaf and interior page to a fresh location, records the freed pages, rewrites the meta page, and syncs twice. One cache over leaf and interior pages stands for the page cache of a memory-mapped engine; leaf pages holding the resident top are held in DRAM ({:.3} GB). The page map is static: no page splits or merges over the replay.",
            LEAF_ENTRIES,
            MEAN_ENTRY_BYTES,
            FILL,
            FANOUT,
            g.height(),
            g.levels[0],
            g.total_pages() as f64 * pbt_store::PAGE as f64 / 1e9,
            btm.dram_bytes as f64 / GB,
        );
        let _ = writeln!(
            md,
            "- measured, B+tree, per block: {:.1} pages rewritten copy-on-write; with no cache {:.1} leaf-page reads and {:.1} interior-page reads, execution reads included; {:.1} leaf pages and {:.1} interior pages copied, {:.2} MB held by one commit, {:.0} fsyncs",
            btm.rand_page_writes as f64 / b,
            leaf_reads as f64 / b,
            btc.accesses.saturating_sub(leaf_reads) as f64 / b,
            btc.btree_leaf_page_writes as f64 / b,
            btc.btree_interior_page_writes as f64 / b,
            btm.rand_page_writes as f64 / b * pbt_store::PAGE as f64 / MB,
            btm.fsyncs as f64 / b,
        );
    }
    let _ = writeln!(
        md,
        "- page log, MODELED from the page store's run: the same merkle and value-store pages appended to a log in sequence instead of rewritten in place, found through a page index in DRAM ({:.2} GB: {} B per page over {} pages, in place of the bottom branch nodes and the bucket map), compacted when half the log is garbage (each garbage page read twice and rewritten once, amortized; relocations update the DRAM index for free), two syncs per block. page log: {:.1} pages appended per block, each superseding one; space up to 2x the live pages.",
        page_index_bytes as f64 / gbk,
        PAGE_INDEX_ENTRY_BYTES,
        2 * leaf_pages * k,
        pages_appended as f64 / b,
    );
    if let Some(c) = &page_lsm_cost {
        let per_block = (c.stream_updates * k) as f64 / (facts.warmup + facts.blocks).max(1) as f64;
        let _ = writeln!(
            md,
            "- page LSM, MODELED from the page store's run: the same pages as {:.0} B values of a leveled merge tree keyed by page id, with the engine of the index above; what each level absorbs is MEASURED from the {} page writes of the replay ({:.0} per block, warm-up included). page write amplification {:.1} per page update ({:.0} KB written and read back by compaction per page written, plus one WAL copy).",
            PAGE_LSM_ENTRY_BYTES,
            c.stream_updates,
            per_block,
            c.amplification,
            c.amplification * PAGE_LSM_ENTRY_BYTES / 1024.0,
        );
        lsm_level_table(&mut md, c, per_block);
    }
    let _ = writeln!(
        md,
        "- log store, lazier garbage collection, MODELED: compacting when two thirds of the log is garbage instead of half moves one live record per two superseded instead of one per one. relocations {:.1} per block at g = 2/3 ({:.1} at g = 1/2), index write amplification {:.1} ({:.1} at g = 1/2), compaction reading {:.2} MB and rewriting {:.2} MB per block ({:.2} and {:.2} at g = 1/2); space up to 3x the live records instead of 2x.",
        gc23.1.relocations as f64 / b,
        ac.relocations as f64 / b,
        gc23.2,
        amp,
        gc23.0.seq_bytes_read as f64 / b / MB,
        gc23.0.seq_bytes_written.saturating_sub(ac.appended_bytes) as f64 / b / MB,
        arm.seq_bytes_read as f64 / b / MB,
        arm.seq_bytes_written.saturating_sub(ac.appended_bytes) as f64 / b / MB,
    );
    let _ = writeln!(
        md,
        "- LSM, page-keyed, MODELED: the LSM of records with its key laid out as (page level, the 6-level-aligned path prefix, the position within that page) instead of the path itself, so that a 4 KB data block holds the records of one NOMT-style page. Its writes are those of the LSM of records; its reads are the page store's page cache at the LSM's fixed DRAM, plus the displaced leaves ({:.1} per block). The page store keeps the records of a sparse page in no page and rebuilds them from the value page it reads for the changed key; here they sit in the block that read fetches, so the {:.1} elided-page touches per block add no read of their own. The page cache is borrowed from the page store, which refreshes a fully written page without a read and drops an elided one, while a block cache fills only on reads: a favourable stand-in.",
        ac.displaced_leaf_splits as f64 / b,
        pvs.uncharged_elided_touches as f64 / b,
    );
    let _ = writeln!(
        md,
        "- limits of the modeled state: a written stem and a written account's header hold only what the sample writes, so paths inside them are short; fetched slot counts include the slots numbered below 64, which live in the account header and not in the storage zone; an untouched stem is one leaf; the code zone is one leaf. The missing header slots were estimated at under 0.6% of branch records; the others are not quantified."
    );
    if exec_present + exec_absent == 0 {
        let _ = writeln!(
            md,
            "- not charged to either store: reads a block makes of values it does not change. An uncached one costs the page store one leaf-page read, and the log store one record read plus, with its index on disk, one index read."
        );
    } else {
        let _ = writeln!(
            md,
            "- not charged to either store: reads of contract code, which the dataset does not list. In the tables below, reads include the execution reads; the CSV column keeps its old name."
        );
    }
    let _ = writeln!(
        md,
        "- `cheapest` compares six families, page, log, LSM, B+tree, page log and page LSM, each represented by the variant with the best worst case, and names one only if it beats every other family by {TOO_CLOSE}x or more whichever end of its own range each is at; the ratio printed is against the runner-up named.\n"
    );

    // How fast misses fall inside the range the replay resolves exactly.
    let _ = writeln!(
        md,
        "### {} per block against cache size, exact range only\n",
        if exec_present + exec_absent == 0 {
            "Write-path reads"
        } else {
            "Reads (write path and execution)"
        }
    );
    let _ = writeln!(
        md,
        "Each row halves the cache. This is the part of the curve the replay measures; the tables below go beyond it.\n"
    );
    let _ = writeln!(
        md,
        "| share of the exact range | log: cached records | GB | reads/blk | page: cached pages | GB | reads/blk |"
    );
    let _ = writeln!(md, "|---|---|---|---|---|---|---|");
    for shift in (0..=6).rev() {
        let (ca, cp) = (ac.exact_upto >> shift, pvs.exact_upto >> shift);
        let _ = writeln!(
            md,
            "| 1/{} | {} | {:.3} | {:.1} | {} | {:.3} | {:.1} |",
            1u32 << shift,
            ca * k,
            (ca * filter.cache_entry_bytes) as f64 / gbk,
            ac.bounds_at(ca).1 as f64 / b,
            cp * k,
            (cp * pvs.entry_bytes) as f64 / gbk,
            pvs.bounds_at(cp).1 as f64 / b,
        );
    }
    let _ = writeln!(
        md,
        "\nWith no cache at all: {:.1} reads per block for the log store, {:.1} for the page store. Of the log store's, {:.1} are of records untouched since the replay began; of the page store's, {:.1}.\n",
        ac.accesses as f64 / b,
        pvs.accesses as f64 / b,
        ac.cold as f64 / b,
        pvs.cold as f64 / b,
    );

    const COLS: usize = 10;
    let names: [String; COLS] = [
        "page, NOMT value store".into(),
        "log, index in DRAM".into(),
        format!("log, index on disk ({} B)", filter.resident_bytes),
        "LSM, node cache".into(),
        "LSM, block cache".into(),
        "B+tree, copy-on-write".into(),
        "log, index on disk, GC at 2/3".into(),
        "LSM, page-keyed".into(),
        "page log, GC at 1/2".into(),
        "page LSM".into(),
    ];
    // A cell at both ends of the range; `d` is already at 1/k.
    let span_of =
        |prof: &DeviceProfile, price: Price, f: &dyn Fn(Bound) -> Option<AtBudget>| -> Span {
            let cost = |x: Option<AtBudget>| x.map(|x| io_cost(&x.m, prof, blocks, price));
            Some((cost(f(Bound::Lo))?, cost(f(Bound::Hi))?))
        };
    // `di[0]` is the index of the record log at g = 1/2, `di[1]` at g = 2/3.
    let costs = |prof: &DeviceProfile,
                 price: Price,
                 d: u64,
                 di: [DiskIndex; 2],
                 lr: LsmRecords|
     -> [Span; COLS] {
        let d = d / k;
        let both = |f: &dyn Fn(Bound) -> Option<AtBudget>| -> Span { span_of(prof, price, f) };
        [
            both(&|bd| page_with_value_store(&prm, pc, &pvs_true, d, blocks, bd)),
            both(&|bd| at_budget(&arm, &ac_true, d, bd, displaced)),
            both(&|bd| log_with_disk_index(&arm, &ac_true, d, di[0], bd)),
            both(&|bd| {
                lsm_of_records(
                    lsm_updates,
                    ac.displaced_leaf_splits,
                    &lsm_node,
                    d,
                    lr,
                    bd,
                    blocks,
                )
            }),
            lsm_block.as_ref().and_then(|c| {
                both(&|bd| {
                    lsm_of_records(lsm_updates, ac.displaced_leaf_splits, c, d, lr, bd, blocks)
                })
            }),
            bt_true
                .as_ref()
                .and_then(|(m, c)| both(&|bd| at_budget(m, c, d, bd, Extra::default()))),
            both(&|bd| log_with_disk_index(&gc23.0, &gc23.1, d, di[1], bd)),
            both(&|bd| lsm_of_records(lsm_updates, lsm_paged_extra, &lsm_paged, d, lr, bd, blocks)),
            both(&|bd| page_log(pages_appended, &pl_curve, d, 0.5, bd, blocks)),
            page_lsm.and_then(|pl| {
                both(&|bd| lsm_of_records(pages_appended, 0, &page_lsm_curve, d, pl, bd, blocks))
            }),
        ]
    };
    let fmt = |x: Span| match x {
        Some((lo, hi)) if (hi - lo).abs() < 0.5 => format!("{hi:.0}"),
        Some((lo, hi)) => format!("{lo:.0}-{hi:.0}"),
        None => "-".to_string(),
    };

    // Whole-GB budgets, plus the two budgets at which a disk-index log
    // store's cache is exactly what the replay resolves.
    let mut budgets_gb = vec![1.0f64, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0];
    for di in [filter, fingerprint] {
        budgets_gb.push((fixed_of(di) + ac.exact_upto * di.cache_entry_bytes) as f64 / gbk);
    }
    budgets_gb.sort_by(|a, b| a.partial_cmp(b).unwrap());
    for prof in PROFILES.iter() {
        for (price, label) in [(Price::PerIo, "per IO"), (Price::Bandwidth, "by bytes")] {
            let _ = writeln!(
                md,
                "### IO cost per block, mainnet replay, profile `{}`, reads priced {}\n",
                prof.name, label
            );
            let _ = writeln!(md, "| DRAM GB | {} | cheapest |", names.join(" | "));
            let _ = writeln!(md, "|{}", "---|".repeat(COLS + 2));
            for &gb in &budgets_gb {
                let cells = costs(prof, price, (gb * GB) as u64, [filter, gc23_di], lr);
                let _ = writeln!(
                    md,
                    "| {} | {} | {} |",
                    if gb.fract() == 0.0 {
                        format!("{gb}")
                    } else {
                        format!("{gb:.2}")
                    },
                    cells
                        .iter()
                        .map(|c| fmt(*c))
                        .collect::<Vec<_>>()
                        .join(" | "),
                    verdict(&cells, &names)
                );
            }
            let _ = writeln!(md);
        }
    }

    // The counts behind the cost cells, for timing them on a real device
    // (`pbt-char iobench`). `hi` bound: every unknown read is a miss.
    let _ = writeln!(md, "### IO mix per block, for timed IO (`hi` bound)\n");
    let _ = writeln!(
        md,
        "| DRAM GB | store | random 4 KB reads | random 512 B reads | random 4 KB writes | sequential bytes written | sequential bytes read | fsyncs | execution-read misses |"
    );
    let _ = writeln!(md, "|---|---|---|---|---|---|---|---|---|");
    for gb in [4.0f64, 8.0, 16.0, 32.0] {
        let d = (gb * GB) as u64 / k;
        let rows: Vec<(&str, Option<AtBudget>, Option<&CacheProfile>)> = vec![
            (
                "page",
                page_with_value_store(&prm, pc, &pvs_true, d, blocks, Bound::Hi),
                Some(&pvs_true),
            ),
            (
                "log, index on disk (1.5 B)",
                log_with_disk_index(&arm, &ac_true, d, filter, Bound::Hi),
                Some(&ac_true),
            ),
            (
                "LSM, node cache",
                lsm_of_records(
                    lsm_updates,
                    ac.displaced_leaf_splits,
                    &lsm_node,
                    d,
                    lr,
                    Bound::Hi,
                    blocks,
                ),
                Some(&lsm_node),
            ),
            (
                "LSM, block cache",
                lsm_block.as_ref().and_then(|c| {
                    lsm_of_records(
                        lsm_updates,
                        ac.displaced_leaf_splits,
                        c,
                        d,
                        lr,
                        Bound::Hi,
                        blocks,
                    )
                }),
                lsm_block.as_ref(),
            ),
            (
                "B+tree, copy-on-write",
                bt_true
                    .as_ref()
                    .and_then(|(m, c)| at_budget(m, c, d, Bound::Hi, Extra::default())),
                bt_true.as_ref().map(|(_, c)| c),
            ),
            (
                "log, index on disk, GC at 2/3",
                log_with_disk_index(&gc23.0, &gc23.1, d, gc23_di, Bound::Hi),
                Some(&gc23.1),
            ),
            (
                "LSM, page-keyed",
                lsm_of_records(
                    lsm_updates,
                    lsm_paged_extra,
                    &lsm_paged,
                    d,
                    lr,
                    Bound::Hi,
                    blocks,
                ),
                Some(&lsm_paged),
            ),
            (
                "page log, GC at 1/2",
                page_log(pages_appended, &pl_curve, d, 0.5, Bound::Hi, blocks),
                Some(&pl_curve),
            ),
            (
                "page LSM",
                page_lsm.and_then(|pl| {
                    lsm_of_records(pages_appended, 0, &page_lsm_curve, d, pl, Bound::Hi, blocks)
                }),
                Some(&page_lsm_curve),
            ),
        ];
        for (name, x, prof) in rows {
            let (Some(x), Some(prof)) = (x, prof) else {
                continue;
            };
            // Execution reads are needed while the block executes, one
            // after another unless prefetched; the rest at commit, at once.
            let exec_misses = prof.tag_bounds_at(x.cap).1 as f64;
            let m = &x.m;
            let bk = blocks.max(1) as f64;
            // Reads are 4 KB pages or sector-rounded records: split the
            // count so that the bytes match, taking records as 512 B.
            let (n, by) = (m.page_reads as f64, m.read_bytes as f64);
            let r4k = ((by - 512.0 * n) / 3584.0).clamp(0.0, n);
            let _ = writeln!(
                md,
                "| {gb:.0} | {name} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.2} | {:.0} |",
                r4k / bk,
                (n - r4k) / bk,
                m.rand_page_writes as f64 / bk,
                m.seq_bytes_written as f64 / bk,
                m.seq_bytes_read as f64 / bk,
                m.fsyncs as f64 / bk,
                exec_misses / bk,
            );
        }
    }
    let _ = writeln!(md);

    // The index engine is assumed, so show what the assumption is worth.
    let _ = writeln!(
        md,
        "### Sensitivity to the index engine's write amplification, 16 GB\n"
    );
    let _ = writeln!(
        md,
        "The log store with its index on disk ({} B per record in DRAM), IO cost per block. {:.1} is the amplification the tables above use, the model's {:.1} scaled by the RocksDB measurement.\n",
        filter.resident_bytes, amp, amp_model
    );
    let _ = writeln!(
        md,
        "| write amplification | `{0}` per IO | `{0}` by bytes | `{1}` per IO | `{1}` by bytes |",
        PROFILES[0].name, PROFILES[1].name
    );
    let _ = writeln!(md, "|---|---|---|---|---|");
    let d16 = (16.0 * GB) as u64;
    let combos: Vec<(&DeviceProfile, Price)> = PROFILES
        .iter()
        .take(2)
        .flat_map(|p| [(p, Price::PerIo), (p, Price::Bandwidth)])
        .collect();
    let grid: Vec<f64> = if lsm_cost.is_some() {
        vec![amp, amp_model, 5.0, 30.0]
    } else {
        vec![amp / 3.0, amp, amp * 2.0]
    };
    for a in grid {
        let di = [on_disk(1.5, a), gc23_di];
        let cells: Vec<String> = combos
            .iter()
            .map(|(p, price)| fmt(costs(p, *price, d16, di, lr)[2]))
            .collect();
        let _ = writeln!(md, "| {:.1} | {} |", a, cells.join(" | "));
    }
    let cells: Vec<String> = combos
        .iter()
        .map(|(p, price)| fmt(costs(p, *price, d16, [filter, gc23_di], lr)[0]))
        .collect();
    let _ = writeln!(
        md,
        "| page store, for comparison | {} |\n",
        cells.join(" | ")
    );

    // The record engine of the LSM baseline is assumed in the same way.
    let _ = writeln!(
        md,
        "### Sensitivity to the record engine's write amplification, 16 GB\n"
    );
    let _ = writeln!(
        md,
        "The LSM of records, IO cost per block. {:.1} is the amplification the tables above use, the model's {:.1} scaled by the RocksDB measurement. Entry-size uncertainty maps onto this axis: bytes per update are entry bytes times amplification.\n",
        lsm_amp, lsm_amp_model
    );
    let _ = writeln!(
        md,
        "| write amplification | `{0}` per IO | `{0}` by bytes | `{1}` per IO | `{1}` by bytes |",
        PROFILES[0].name, PROFILES[1].name
    );
    let _ = writeln!(md, "|---|---|---|---|---|");
    let grid: Vec<f64> = if lsm_records_cost.is_some() {
        vec![lsm_amp, 5.0, 15.0, 30.0]
    } else {
        vec![lsm_amp / 3.0, lsm_amp, lsm_amp * 2.0]
    };
    for a in grid {
        let lr2 = LsmRecords { amp: a, ..lr };
        for (col, name) in [
            (3usize, "node cache"),
            (4, "block cache"),
            (7, "page-keyed"),
        ] {
            let cells: Vec<String> = combos
                .iter()
                .map(|(p, price)| fmt(costs(p, *price, d16, [filter, gc23_di], lr2)[col]))
                .collect();
            let _ = writeln!(md, "| {:.1}, {} | {} |", a, name, cells.join(" | "));
        }
    }
    let _ = writeln!(md);

    // Garbage collection is a knob both logs have: fewer relocations and
    // compaction reads for more space.
    let _ = writeln!(md, "### Sensitivity to garbage collection, 16 GB\n");
    let _ = writeln!(
        md,
        "The record log with its index on disk and the page log, IO cost per block, compacted when a fraction g of the log is garbage; space is up to 1/(1-g) of the live data. The tables above use g = 1/2 for the page log, and 1/2 and 2/3 for the record log.\n"
    );
    let _ = writeln!(
        md,
        "| g | store | `{0}` per IO | `{0}` by bytes | `{1}` per IO | `{1}` by bytes | space |",
        PROFILES[0].name, PROFILES[1].name
    );
    let _ = writeln!(md, "|---|---|---|---|---|---|---|");
    for g in [0.5f64, 2.0 / 3.0, 0.8] {
        let (mg, cg, ag) = log_at_gc(&gc_base, g);
        let dig = on_disk(1.5, ag);
        let d = d16 / k;
        let log_cells: Vec<String> = combos
            .iter()
            .map(|(p, price)| {
                fmt(span_of(p, *price, &|bd| {
                    log_with_disk_index(&mg, &cg, d, dig, bd)
                }))
            })
            .collect();
        let _ = writeln!(
            md,
            "| {g:.2} | log, index on disk (index amplification {ag:.1}) | {} | {:.1}x |",
            log_cells.join(" | "),
            1.0 / (1.0 - g)
        );
        let pl_cells: Vec<String> = combos
            .iter()
            .map(|(p, price)| {
                fmt(span_of(p, *price, &|bd| {
                    page_log(pages_appended, &pl_curve, d, g, bd, blocks)
                }))
            })
            .collect();
        let _ = writeln!(
            md,
            "| {g:.2} | page log | {} | {:.1}x |",
            pl_cells.join(" | "),
            1.0 / (1.0 - g)
        );
    }
    let _ = writeln!(md);

    let mut curves = vec![
        ("page with NOMT value store".to_string(), pvs_true.clone()),
        ("log with index in DRAM".to_string(), ac_true.clone()),
        (
            format!("log with index on disk ({} B)", filter.resident_bytes),
            ac_true
                .with_index_entry_bytes(filter.resident_bytes)
                .with_entry_bytes(filter.cache_entry_bytes)
                .with_fixed_extra(filter.fixed_bytes),
        ),
        ("LSM of records, node cache".to_string(), lsm_node.clone()),
    ];
    if let Some(c) = &lsm_block {
        curves.push(("LSM of records, block cache".to_string(), c.clone()));
    }
    if let Some((_, c)) = &bt_true {
        curves.push(("B+tree, copy-on-write".to_string(), c.clone()));
    }
    curves.push((
        "log with index on disk, GC at 2/3".to_string(),
        gc23.1
            .with_index_entry_bytes(gc23_di.resident_bytes)
            .with_entry_bytes(gc23_di.cache_entry_bytes)
            .with_fixed_extra(gc23_di.fixed_bytes),
    ));
    curves.push(("LSM of records, page-keyed".to_string(), lsm_paged.clone()));
    curves.push(("page log".to_string(), pl_curve.clone()));
    curves.push(("page LSM".to_string(), page_lsm_curve.clone()));
    let csv = curves_csv_scaled(&curves, blocks, k);
    (md, csv)
}

/// Full curves, one row per (store, capacity): for plotting and for
/// re-deriving any budget the report did not tabulate.
fn cache_sweep_csv(rows: &[Row], blocks: u64) -> String {
    let curves: Vec<(String, CacheProfile)> = ["append-readback", "paged-pos", "paged-struct"]
        .iter()
        .filter_map(|prefix| profiled(rows, prefix))
        .map(|(r, c)| (r.name.clone(), c.clone()))
        .collect();
    curves_csv(&curves, blocks)
}

fn curves_csv(curves: &[(String, CacheProfile)], blocks: u64) -> String {
    curves_csv_scaled(curves, blocks, 1)
}

/// `k`: the curves are of one key-space subtree in `k`; capacities and
/// DRAM are scaled up to the whole (the per-block reads already are, by
/// passing `blocks / k`).
fn curves_csv_scaled(curves: &[(String, CacheProfile)], blocks: u64, k: u64) -> String {
    let mut csv = String::from(
        "store,capacity_entries,dram_mb,write_path_reads_per_block_lo,write_path_reads_per_block_hi\n",
    );
    let b = blocks.max(1) as f64;
    for (name, c) in curves {
        let mut caps: Vec<u64> = vec![0];
        let mut x = 1.0f64;
        while (x as u64) < c.cacheable_entries {
            caps.push(x as u64);
            x = (x * 1.08).max(x + 1.0);
        }
        caps.push(c.exact_upto.min(c.cacheable_entries));
        caps.push(c.cacheable_entries);
        caps.sort_unstable();
        caps.dedup();
        for cap in caps {
            let (lo, hi) = c.bounds_at(cap);
            let _ = writeln!(
                csv,
                "{},{},{:.3},{:.3},{:.3}",
                name,
                cap * k,
                ((c.fixed_dram_bytes + cap * c.entry_bytes) * k) as f64 / MB,
                lo as f64 / b,
                hi as f64 / b
            );
        }
    }
    csv
}

fn report(rows: &[Row], blocks: u64, cuts: &[u32], resident: u32) -> String {
    let mut md = String::new();
    let _ = writeln!(
        md,
        "# PBT Phase-2: Storage Backends Under a Fixed Commitment\n"
    );
    let _ = writeln!(
        md,
        "> Commitment is PBT for every row: each backend receives the identical\n> record batch the tree emits per block, so any difference is attributable\n> to physical organization alone. `kv-mdbx` is real libmdbx (reads included);\n> `paged` models NOMT's bitbox as read from source (4KB pages, rootless\n> depth-6 subtrie = 126 node slots, in-place hash-addressed buckets, WAL of\n> node-granularity redo diffs); `append` does real sequential writes with\n> counter-modeled compaction. DRAM-resident paged levels: {resident}.\n"
    );

    let _ = writeln!(md, "## Per-block IO (post-genesis, {blocks} blocks)\n");
    let _ = writeln!(
        md,
        "| backend | seq MB/blk | rand 4K writes/blk | page reads/blk | records/blk | lookups/blk | fsyncs/blk | DRAM MB | disk MB | compactions |"
    );
    let _ = writeln!(md, "|---|---|---|---|---|---|---|---|---|---|");
    for r in rows {
        let b = blocks.max(1) as f64;
        let _ = writeln!(
            md,
            "| {} | {:.3} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} | {} | {} | {} |",
            r.name,
            r.m.seq_bytes_written as f64 / b / (1 << 20) as f64,
            r.m.rand_page_writes as f64 / b,
            r.m.page_reads as f64 / b,
            (r.m.record_writes + r.m.record_deletes) as f64 / b,
            r.m.reads_served as f64 / b,
            r.m.fsyncs as f64 / b,
            r.m.dram_bytes >> 20,
            r.m.disk_bytes >> 20,
            r.m.compactions
        );
    }
    let _ = writeln!(
        md,
        "\n`kv-mdbx` is excluded from the modeled-cost tables below: its IO happens\ninside libmdbx and is invisible to these counters, so its seq/rand columns are\nzero by construction and a modeled cost would be meaningless. It is compared\nseparately on real wall-clock and real on-disk size.\n"
    );

    let _ = writeln!(md, "\n## Read pricing sensitivity (nvme)\n");
    let _ = writeln!(
        md,
        "A per-IO read price ignores transfer size; a page store moves 4 KB per hop\nwhile a log store moves one sector. Both prices shown.\n"
    );
    let _ = writeln!(
        md,
        "| backend | per-IO cost/blk | bandwidth cost/blk | read KB/blk |"
    );
    let _ = writeln!(md, "|---|---|---|---|");
    for r in rows.iter().filter(|r| r.name != "kv-mdbx") {
        let _ = writeln!(
            md,
            "| {} | {:.1} | {:.1} | {:.1} |",
            r.name,
            r.m.cost_per_block(&PROFILES[0], blocks),
            r.m.cost_per_block_bw(&PROFILES[0], blocks),
            r.m.read_bytes as f64 / blocks.max(1) as f64 / 1024.0
        );
    }

    for p in PROFILES.iter() {
        let _ = writeln!(md, "\n## Modeled cost, profile `{}`\n", p.name);
        let _ = writeln!(
            md,
            "Units: one sequential 4KB write = 1. rand write {}x, rand read {}x.\n",
            p.c_rand_write, p.c_rand_read
        );
        let _ = writeln!(md, "| backend | cost/block | vs best | DRAM MB |");
        let _ = writeln!(md, "|---|---|---|---|");
        let modeled: Vec<&Row> = rows.iter().filter(|r| r.name != "kv-mdbx").collect();
        let best = modeled
            .iter()
            .map(|r| r.m.cost_per_block(p, blocks))
            .fold(f64::INFINITY, f64::min);
        for r in &modeled {
            let c = r.m.cost_per_block(p, blocks);
            let _ = writeln!(
                md,
                "| {} | {:.1} | {:.2}x | {} |",
                r.name,
                c,
                c / best.max(1e-9),
                r.m.dram_bytes >> 20
            );
        }
        // d* per DRAM ceiling: the argmin is only meaningful at a fixed
        // memory budget, since a deeper cut buys speed with DRAM.
        // The DRAM ceiling has to bind every candidate, not only the split
        // rows: a policy that buys its speed with an unbounded resident set
        // must be able to fail the same constraint.
        for r in &modeled {
            if !r.m.fits_dram(1024 << 20) {
                let _ = writeln!(
                    md,
                    "- NOTE: `{}` needs {} MB resident and does not fit a 1024 MB ceiling",
                    r.name,
                    r.m.dram_bytes >> 20
                );
            }
        }
        for ceiling_mb in [64u64, 256, 1024] {
            let ceiling = ceiling_mb << 20;
            let mut best_cut: Option<(u32, f64)> = None;
            for &cut in cuts {
                if let Some(r) = rows.iter().find(|r| r.name == format!("split(cut={cut})")) {
                    if !r.m.fits_dram(ceiling) {
                        continue;
                    }
                    let c = r.m.cost_per_block(p, blocks);
                    if best_cut.map_or(true, |(_, bc)| c < bc) {
                        best_cut = Some((cut, c));
                    }
                }
            }
            match best_cut {
                Some((cut, c)) => {
                    let _ = writeln!(
                        md,
                        "- DRAM ceiling {ceiling_mb} MB: **d\\* = {cut}** (cost {c:.1}/block)"
                    );
                }
                None => {
                    let _ = writeln!(md, "- DRAM ceiling {ceiling_mb} MB: no configuration fits");
                }
            }
        }
    }

    let _ = writeln!(md, "\n## Real-IO comparison (wall clock, on-disk)\n");
    let _ = writeln!(md, "| backend | commit wall ms/blk | disk MB | basis |");
    let _ = writeln!(md, "|---|---|---|---|");
    for r in rows {
        let basis = if r.name == "kv-mdbx" {
            "real libmdbx, durable sync, reads real"
        } else if r.name.starts_with("paged") {
            "real WAL write+fsync; page IO counter-modeled"
        } else if r.name == "append" {
            "real log write+fsync; compaction counter-modeled"
        } else {
            "counter-modeled"
        };
        let _ = writeln!(
            md,
            "| {} | {:.3} | {} | {} |",
            r.name,
            r.m.wall_ns as f64 / blocks.max(1) as f64 / 1e6,
            r.m.disk_bytes >> 20,
            basis
        );
    }

    let _ = writeln!(md, "\n## Backend detail\n");
    for r in rows {
        if !r.extra.is_empty() {
            let _ = write!(md, "{}", r.extra);
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbt_store::mrc::StackProfile;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn families_come_before_the_comma() {
        assert_eq!(family("page, NOMT value store"), "page");
        assert_eq!(family("log, index on disk (1.5 B)"), "log");
        assert_eq!(family("B+tree, copy-on-write"), "B+tree");
        assert_eq!(family("page"), "page");
    }

    /// With page and log only, the family rule is the rule the two-store
    /// tables used, ratios included (cells from part 0 of the 24.0M run).
    #[test]
    fn verdict_reduces_to_the_two_family_rule() {
        let nm = names(&[
            "page, NOMT value store",
            "log, index in DRAM",
            "log, index on disk (1.5 B)",
            "log, index on disk (4.25 B)",
        ]);
        let cells = [
            Some((21399.0, 21399.0)),
            None,
            Some((6866.0, 11571.0)),
            None,
        ];
        assert_eq!(verdict(&cells, &nm), "log (1.85x to 3.12x over page)");
        let cells = [
            Some((21758.0, 21758.0)),
            None,
            Some((22445.0, 22445.0)),
            None,
        ];
        assert_eq!(
            verdict(&cells, &nm),
            "too close (page by 1.03x to 1.03x over log)"
        );
        let cells = [None, None, Some((1.0, 2.0)), None];
        assert_eq!(verdict(&cells, &nm), "log (others cannot run)");
        let cells = [Some((1.0, 2.0)), None, None, None];
        assert_eq!(verdict(&cells, &nm), "page (others cannot run)");
        // Overlap: the keenest log variant sets how far log could be ahead.
        let cells = [
            Some((9000.0, 11000.0)),
            Some((1500.0, 12000.0)),
            Some((8000.0, 10000.0)),
            None,
        ];
        assert_eq!(
            verdict(&cells, &nm),
            "undecided (log by up to 7.33x, page by up to 1.11x)"
        );
        assert_eq!(verdict(&[None, None, None, None], &nm), "-");
    }

    #[test]
    fn verdict_over_four_families() {
        let nm = names(&[
            "page, NOMT value store",
            "log, index on disk (1.5 B)",
            "LSM, node cache",
            "LSM, block cache",
            "B+tree, copy-on-write",
        ]);
        let cells = [
            Some((100.0, 100.0)),
            Some((60.0, 80.0)),
            Some((50.0, 70.0)),
            Some((40.0, 65.0)),
            Some((200.0, 300.0)),
        ];
        assert_eq!(
            verdict(&cells, &nm),
            "undecided (LSM by up to 2.00x, log by up to 1.08x)"
        );
        let cells = [
            Some((100.0, 100.0)),
            Some((60.0, 80.0)),
            Some((10.0, 20.0)),
            None,
            Some((200.0, 300.0)),
        ];
        assert_eq!(verdict(&cells, &nm), "LSM (3.00x to 8.00x over log)");
        let cells = [
            Some((100.0, 100.0)),
            Some((60.0, 80.0)),
            None,
            None,
            Some((20.0, 30.0)),
        ];
        assert_eq!(verdict(&cells, &nm), "B+tree (2.00x to 4.00x over log)");
    }

    #[test]
    fn page_log_arithmetic() {
        let mut p = StackProfile::new();
        p.access(1, 4096);
        p.access(2, 4096);
        p.access(1, 4096);
        let curve = p.curve(4112, 0, 10);
        let x = page_log(100, &curve, 100_000, 0.5, Bound::Hi, 3).unwrap();
        assert_eq!(x.m.page_reads, 2);
        assert_eq!(x.m.seq_bytes_written, 100 * 4096 * 2);
        assert_eq!(x.m.seq_bytes_read, 100 * 4096 * 2);
        assert_eq!(x.m.fsyncs, PAGE_LOG_FSYNCS * 3);
        assert_eq!(x.m.rand_page_writes, 0);
        let lazy = page_log(100, &curve, 100_000, 0.8, Bound::Hi, 3).unwrap();
        assert_eq!(lazy.m.seq_bytes_written, (100.0 * 4096.0 * 1.25) as u64);
        assert_eq!(lazy.m.seq_bytes_read, 100 * 4096 * 5 / 4);
    }

    /// At g = 1/2 the derived window is the measured one; lazier
    /// collection scales relocations and compaction as stated.
    #[test]
    fn log_at_gc_reproduces_the_measured_window_at_one_half() {
        let arm = Metrics {
            seq_bytes_written: 1_000 + 400,
            seq_bytes_read: 800,
            page_reads: 7,
            record_writes: 50,
            ..Metrics::default()
        };
        let mut p = StackProfile::new();
        p.access(1, 512);
        let mut ac = p.curve(96, 0, 100);
        ac.relocations = 20;
        let base = GcBase {
            arm: &arm,
            ac_true: &ac,
            relocations_at_half: 20,
            appended: 1_000,
            garbage: 400,
            stream_updates: 50,
            index_curve: None,
            records: 1_000_000,
            buffer: 1 << 20,
            assumed_amp: 15.0,
        };
        let (m, c, amp) = log_at_gc(&base, 0.5);
        assert_eq!(m.seq_bytes_written, arm.seq_bytes_written);
        assert_eq!(m.seq_bytes_read, arm.seq_bytes_read);
        assert_eq!(m.page_reads, 7);
        assert_eq!(c.relocations, 20);
        assert_eq!(amp, 15.0);
        let (m, c, _) = log_at_gc(&base, 0.8);
        assert_eq!(m.seq_bytes_written, 1_000 + 100);
        assert_eq!(m.seq_bytes_read, 500);
        assert_eq!(c.relocations, 5);
    }

    #[test]
    fn the_index_measurement_scales_the_model_between_its_two_points() {
        let [(r0, f0), (r1, f1)] = INDEX_AMP_ENGINE_OVER_MODEL;
        assert!((index_amp_measured(10.0, r0) - 10.0 * f0).abs() < 1e-9);
        assert!((index_amp_measured(10.0, r1) - 10.0 * f1).abs() < 1e-9);
        let mid = index_amp_measured(10.0, (r0 + r1) / 2.0);
        assert!((mid - 10.0 * (f0 + f1) / 2.0).abs() < 1e-9);
        assert_eq!(index_amp_measured(10.0, 0.0), 10.0 * f0);
        assert_eq!(index_amp_measured(10.0, 5.0), 10.0 * f1);
    }

    #[test]
    fn verdict_over_six_families() {
        let nm = names(&[
            "page, NOMT value store",
            "log, index on disk (1.5 B)",
            "LSM, node cache",
            "B+tree, copy-on-write",
            "LSM, page-keyed",
            "page log, GC at 1/2",
            "page LSM",
        ]);
        let cells = [
            Some((100.0, 100.0)),
            Some((60.0, 80.0)),
            Some((50.0, 70.0)),
            Some((300.0, 300.0)),
            Some((30.0, 40.0)),
            Some((90.0, 95.0)),
            None,
        ];
        assert_eq!(verdict(&cells, &nm), "LSM (1.50x to 2.67x over log)");
        assert_eq!(family("page log, GC at 1/2"), "page log");
        assert_eq!(family("page LSM"), "page LSM");
    }

    #[test]
    fn lsm_of_records_arithmetic() {
        let mut p = StackProfile::new();
        p.access(1, 4096);
        p.access(2, 4096);
        p.access(1, 4096);
        // Two cold accesses, one warm at distance 1.
        let curve = p.curve(100, 0, 10);
        let cfg = LsmRecords {
            entry_bytes: 80.0,
            amp: 10.0,
        };
        let x = lsm_of_records(10, 0, &curve, 1000, cfg, Bound::Hi, 2).unwrap();
        assert_eq!(x.cap, 10);
        assert_eq!(x.m.page_reads, 2);
        assert_eq!(x.m.read_bytes, 2 * 4096);
        assert_eq!(x.m.seq_bytes_written, 10 * 80 * 11);
        assert_eq!(x.m.seq_bytes_read, 10 * 80 * 10);
        assert_eq!(x.m.fsyncs, 2);
        assert_eq!(x.m.rand_page_writes, 0);
        let lo = lsm_of_records(10, 3, &curve, 1000, cfg, Bound::Lo, 2).unwrap();
        assert_eq!(lo.m.page_reads, 3);
        assert!(lsm_of_records(
            10,
            0,
            &curve.with_fixed_extra(2000),
            1000,
            cfg,
            Bound::Hi,
            2
        )
        .is_none());
    }
}
