//! Phase-2 harness: one workload, several physical backends, commitment
//! held fixed at PBT. Sweeps the paged/append cut depth to locate `d*`.

use crate::gen::{Params, World, Write as StateWrite};
use crate::mainnet;
use pbt_ref::tree::{CommitSink, PbtTree, ReadPath, RecordKind, RewriteStats, ZoneLabel};
use pbt_store::append::BranchHashPolicy;
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
        // the two stores that can be judged against that full state run:
        // the page store told what the full state looks like, and the log
        // store whose branch reads are profiled.
        backends.push(Box::new(
            PagedBackend::with_packing(None, cfg.resident_levels, Packing::Positional)
                .with_shape(mn.shape.clone()),
        ));
        backends.push(Box::new(AppendBackend::with_policy(
            None,
            cfg.resident_levels * 6,
            BranchHashPolicy::ReadBack,
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
            // Index updates from here on drive the on-disk index model.
            if cfg.mainnet.is_some() && b.name().starts_with("append-readback") {
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

fn index_levels(records: u64, buffer: u64) -> f64 {
    (records as f64 * INDEX_ENTRY_DISK_BYTES / buffer as f64)
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

    let levels = index_levels(records, buffer);
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
    let amp = lsm_cost.as_ref().map_or(assumed_amp, |c| c.amplification);
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

    let _ = writeln!(md, "\n## Mainnet replay under a full node's DRAM\n");
    let _ = writeln!(
        md,
        "> Nature: MAINNET write stream, replayed on a tree whose untouched state is\n> MODELED, costed with the COUNTER model. Which keys each block writes, and\n> whether each write creates, rewrites or deletes, is real. The rest of the\n> state is not loaded: beside every written path, wherever the full state has\n> keys on the other side of a branch, a stand-in is inserted, so paths have\n> the depth they would have on mainnet. Where the full state has keys is\n> taken from its size and EIP-8297's hash placement, not from the keys\n> themselves. The page side is NOMT as built; the log side with its index on\n> disk is an ASSUMED engine. {}\n",
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
            "- inside every index-on-disk figure, the index engine MODELED from this replay: a leveled merge tree over {:.0} GB of 16-byte entries, a {} MB write buffer, levels growing by {}x down to one that holds every entry. How much each level absorbs is MEASURED from the {} index updates of the replay ({:.0} per block, warm-up included): an entry updated again while it sits in the buffer or an upper level never moves further down. The {:.1} relocations per update are taken never to be absorbed. Write amplification {:.1} per index update ({:.0} B written and read), against {:.0} for the assumed engine before. That costs {:.0} per block on `{}` and {:.0} on `{}`. Every lookup the cache cannot answer costs one more 4 KB read (ASSUMED, as before). A log whose branches are found without an index pays neither for branches.",
            index_disk / gbk,
            INDEX_BUFFER_BYTES >> 20,
            INDEX_SIZE_RATIO,
            c.stream_updates,
            per_block,
            c.relocation_share,
            amp,
            filter.bytes_per_update,
            assumed_amp,
            upd_pages * (PROFILES[0].c_seq4k_write + PROFILES[0].c_seq4k_read),
            PROFILES[0].name,
            upd_pages * (PROFILES[1].c_seq4k_write + PROFILES[1].c_seq4k_read),
            PROFILES[1].name,
        );
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
        "- `cheapest` names a store only if it wins by {TOO_CLOSE}x or more whichever end of its own range each store is at. The log store is represented by the variant with the best worst case.\n"
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

    type Span = Option<(f64, f64)>;
    let costs =
        |prof: &DeviceProfile, price: Price, d: u64, di: [DiskIndex; 2]| -> (Span, [Span; 3]) {
            let d = d / k;
            let cost = |x: Option<AtBudget>| x.map(|x| io_cost(&x.m, prof, blocks, price));
            let both = |f: &dyn Fn(Bound) -> Option<AtBudget>| -> Span {
                Some((cost(f(Bound::Lo))?, cost(f(Bound::Hi))?))
            };
            (
                both(&|bd| page_with_value_store(&prm, pc, &pvs_true, d, blocks, bd)),
                [
                    both(&|bd| at_budget(&arm, &ac_true, d, bd, displaced)),
                    both(&|bd| log_with_disk_index(&arm, &ac_true, d, di[0], bd)),
                    both(&|bd| log_with_disk_index(&arm, &ac_true, d, di[1], bd)),
                ],
            )
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
            let _ = writeln!(
                md,
                "| DRAM GB | page, NOMT value store | log, index in DRAM | log, index on disk ({} B) | log, index on disk ({} B) | cheapest |",
                filter.resident_bytes, fingerprint.resident_bytes
            );
            let _ = writeln!(md, "|---|---|---|---|---|---|");
            for &gb in &budgets_gb {
                let (page, logs) = costs(prof, price, (gb * GB) as u64, [filter, fingerprint]);
                let running: Vec<(f64, f64)> = logs.iter().flatten().copied().collect();
                let verdict = match (page, running.is_empty()) {
                    (None, true) => "-".to_string(),
                    (None, false) => "log (page cannot run)".to_string(),
                    (Some(_), true) => "page (log cannot run)".to_string(),
                    (Some((p_lo, p_hi)), false) => {
                        // Each store's unknown reads are its own, so its
                        // range is independent of the other's: the least a
                        // winner wins by is its `hi` against the loser's `lo`.
                        let by_hi =
                            |a: &&(f64, f64), b: &&(f64, f64)| a.1.partial_cmp(&b.1).unwrap();
                        let by_lo =
                            |a: &&(f64, f64), b: &&(f64, f64)| a.0.partial_cmp(&b.0).unwrap();
                        let safest = *running.iter().min_by(by_hi).unwrap();
                        let keenest = *running.iter().min_by(by_lo).unwrap();
                        if p_lo / safest.1 >= TOO_CLOSE {
                            format!("log ({:.2}x to {:.2}x)", p_lo / safest.1, p_hi / safest.0)
                        } else if keenest.0 / p_hi >= TOO_CLOSE {
                            format!("page ({:.2}x to {:.2}x)", keenest.0 / p_hi, safest.1 / p_lo)
                        } else if safest.1 <= p_lo {
                            format!(
                                "too close (log by {:.2}x to {:.2}x)",
                                p_lo / safest.1,
                                p_hi / safest.0
                            )
                        } else if keenest.0 >= p_hi {
                            format!(
                                "too close (page by {:.2}x to {:.2}x)",
                                keenest.0 / p_hi,
                                safest.1 / p_lo
                            )
                        } else {
                            // The ranges overlap. If neither store could be
                            // ahead by the threshold even at its best, the
                            // cell is close rather than open.
                            let (log_best, page_best) = (p_hi / keenest.0, safest.1 / p_lo);
                            let word = if log_best.max(page_best) < TOO_CLOSE {
                                "too close"
                            } else {
                                "undecided"
                            };
                            format!("{word} (log by up to {log_best:.2}x, page by up to {page_best:.2}x)")
                        }
                    }
                };
                let _ = writeln!(
                    md,
                    "| {} | {} | {} | {} | {} | {} |",
                    if gb.fract() == 0.0 {
                        format!("{gb}")
                    } else {
                        format!("{gb:.2}")
                    },
                    fmt(page),
                    fmt(logs[0]),
                    fmt(logs[1]),
                    fmt(logs[2]),
                    verdict
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
        let rows = [
            (
                "page",
                page_with_value_store(&prm, pc, &pvs_true, d, blocks, Bound::Hi),
                &pvs_true,
            ),
            (
                "log, index on disk (1.5 B)",
                log_with_disk_index(&arm, &ac_true, d, filter, Bound::Hi),
                &ac_true,
            ),
        ];
        for (name, x, prof) in rows {
            let Some(x) = x else { continue };
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
        "The log store with its index on disk ({} B per record in DRAM), IO cost per block. {:.1} is the amplification the tables above use.\n",
        filter.resident_bytes, amp
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
        vec![amp, 5.0, 15.0, 30.0]
    } else {
        vec![amp / 3.0, amp, amp * 2.0]
    };
    for a in grid {
        let di = [on_disk(1.5, a), on_disk(4.25, a)];
        let cells: Vec<String> = combos
            .iter()
            .map(|(p, price)| fmt(costs(p, *price, d16, di).1[1]))
            .collect();
        let _ = writeln!(md, "| {:.1} | {} |", a, cells.join(" | "));
    }
    let cells: Vec<String> = combos
        .iter()
        .map(|(p, price)| fmt(costs(p, *price, d16, [filter, fingerprint]).0))
        .collect();
    let _ = writeln!(
        md,
        "| page store, for comparison | {} |\n",
        cells.join(" | ")
    );

    let csv = curves_csv_scaled(
        &[
            ("page with NOMT value store".to_string(), pvs_true.clone()),
            ("log with index in DRAM".to_string(), ac_true.clone()),
            (
                format!("log with index on disk ({} B)", filter.resident_bytes),
                ac_true
                    .with_index_entry_bytes(filter.resident_bytes)
                    .with_entry_bytes(filter.cache_entry_bytes)
                    .with_fixed_extra(filter.fixed_bytes),
            ),
        ],
        blocks,
        k,
    );
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
