//! pbt-char: Phase-1 rewrite-frequency characterization harness for the
//! EIP-8297 Partitioned Binary Tree.
//!
//! Subcommands:
//!   synth  [--seed N] [--eoas N] [--contracts N] [--templates N]
//!          [--blocks N] [--ops-per-block N] [--out DIR] [--trace-out FILE]
//!   replay --trace FILE [--out DIR] [--label NAME]
//!
//! The trace format is key-level (one derived write per line), so a future
//! mainnet-replay tool only needs to emit the same lines:
//!   b                       block boundary
//!   w <hex key> <hex value> state_write (all-zero value = deletion)
//! Convention: the FIRST b-delimited section is the genesis state build and
//! is excluded from rewrite statistics; emit an empty first section (a lone
//! leading `b`) to start measuring from the first real block.

mod alloc;
mod bench;
mod gen;
mod iobench;
mod mainnet;
mod report;
mod rng;

#[global_allocator]
static ALLOC: alloc::Counting = alloc::Counting;

use gen::{Params, World, Write as StateWrite};
use pbt_ref::tree::{PbtTree, RewriteStats};
use std::fs;
use std::io::{BufRead, BufWriter, Write as IoWrite};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("synth") => synth(&args[1..]),
        Some("replay") => replay(&args[1..]),
        Some("bench") => bench_cmd(&args[1..]),
        Some("iobench") => iobench_cmd(&args[1..]),
        _ => {
            eprintln!("usage: pbt-char synth|replay|bench|iobench [flags]");
            std::process::exit(2);
        }
    }
}

/// Timed IO for one per-block mix on a real device; see `iobench.rs`.
///   iobench --file F --size-gb N [--fill] --reads N --read-size B
///           [--reads2 N --read-size2 B]
///           --page-writes N --seq-write-bytes N --seq-read-bytes N
///           --fsyncs N --blocks N --threads N [--label NAME]
fn iobench_cmd(args: &[String]) {
    let file = flag(args, "--file").expect("--file");
    let len = flag_u64(args, "--size-gb", 64) << 30;
    if args.iter().any(|a| a == "--fill") {
        iobench::fill(&file, len);
    }
    let mix = iobench::Mix {
        rand_reads: flag_u64(args, "--reads", 0),
        read_size: flag_u64(args, "--read-size", 4096),
        rand_reads2: flag_u64(args, "--reads2", 0),
        read_size2: flag_u64(args, "--read-size2", 512),
        rand_writes: flag_u64(args, "--page-writes", 0),
        seq_write_bytes: flag_u64(args, "--seq-write-bytes", 0),
        seq_read_bytes: flag_u64(args, "--seq-read-bytes", 0),
        fsyncs: flag_u64(args, "--fsyncs", 1),
    };
    let blocks = flag_u64(args, "--blocks", 100);
    let threads = flag_u64(args, "--threads", 1) as usize;
    let label = flag(args, "--label").unwrap_or_else(|| "mix".into());
    let t = iobench::run(
        &file,
        len,
        &format!("{file}.log"),
        &mix,
        blocks,
        threads,
        0x5eed,
    );
    let mut v = t.per_block_us.clone();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |p: f64| v[((v.len() - 1) as f64 * p) as usize] / 1000.0;
    let mean = v.iter().sum::<f64>() / v.len() as f64 / 1000.0;
    println!(
        "{label}\tdirect={}\tthreads={threads}\tblocks={blocks}\treads={}x{}B+{}x{}B\tpage_writes={}\tseq_w={}\tseq_r={}\tfsyncs={}\tmean_ms={mean:.2}\tp50_ms={:.2}\tp99_ms={:.2}\tread_ms={:.2}\twrite_ms={:.2}",
        t.direct, mix.rand_reads, mix.read_size, mix.rand_reads2, mix.read_size2, mix.rand_writes, mix.seq_write_bytes, mix.seq_read_bytes, mix.fsyncs,
        q(0.5), q(0.99), t.read_us / 1000.0, t.write_us / 1000.0
    );
    if !t.direct {
        eprintln!(
            "[iobench] WARNING: page cache not bypassed on this OS; times are not device times"
        );
    }
}

fn bench_cmd(args: &[String]) {
    let mut p = Params::default();
    p.seed = flag_u64(args, "--seed", p.seed);
    p.eoas = flag_u64(args, "--eoas", p.eoas as u64) as usize;
    p.contracts = flag_u64(args, "--contracts", p.contracts as u64) as usize;
    p.templates = flag_u64(args, "--templates", p.templates as u64) as usize;
    p.blocks = flag_u64(args, "--blocks", p.blocks as u64) as usize;
    p.ops_per_block = flag_u64(args, "--ops-per-block", p.ops_per_block as u64) as usize;
    p.mega_contracts = flag_u64(args, "--mega-contracts", p.mega_contracts as u64) as usize;
    p.same_value_pct = flag_u64(args, "--same-value-pct", p.same_value_pct);
    p.zipf_s = flag_f64(args, "--zipf-s", p.zipf_s);
    p.eoa_zipf_s = flag_f64(args, "--eoa-zipf-s", p.eoa_zipf_s);

    let cuts: Vec<u32> = flag(args, "--cuts")
        .unwrap_or_else(|| "6,12,18,24,30".into())
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().expect("--cuts"))
        .collect();
    let cfg = bench::BenchConfig {
        params: p,
        cuts,
        resident_levels: flag_u64(args, "--resident-levels", 2) as u32,
        reads_per_block: flag_u64(args, "--reads-per-block", 20) as usize,
        value_reads: !has(args, "--proof-reads"),
        batched_reads: has(args, "--batched-reads"),
        with_kv: !has(args, "--no-kv"),
        real_io_dir: flag(args, "--io-dir"),
        warmup_blocks: flag_u64(args, "--warmup-blocks", 0) as usize,
        cache_sweep: has(args, "--cache-sweep"),
        dram_mb: flag(args, "--dram-mb")
            .map(|v| {
                v.split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.parse().expect("--dram-mb"))
                    .collect()
            })
            .unwrap_or_default(),
        mainnet: None,
    };
    let out_dir = flag(args, "--out").unwrap_or_else(|| "results".into());
    let label = flag(args, "--label").unwrap_or_else(|| "phase2".into());
    let out = match flag(args, "--mainnet-diffs") {
        None => bench::run(cfg),
        Some(diffs) => {
            // Replay mainnet state diffs. The state-size figures default to
            // the ethpandaops Lab API's values for 2025-12-13, the day the
            // first sample was taken; pass the day's own figures otherwise.
            let scale = mainnet::Scale {
                accounts: flag_u64(args, "--mainnet-accounts", 334_563_311),
                storage_slots: flag_u64(args, "--mainnet-storage-slots", 1_438_571_749),
                storage_contracts: flag_u64(args, "--mainnet-storage-contracts", 25_000_000),
                code_bytes: flag_u64(args, "--mainnet-code-bytes", 12_391_441_303),
                neighbor_slots: flag(args, "--mainnet-neighbor-slots")
                    .map(|v| v.parse().expect("--mainnet-neighbor-slots N")),
            };
            let slot_counts =
                flag(args, "--mainnet-slot-counts").expect("--mainnet-slot-counts FILE required");
            let batch = flag_u64(args, "--genesis-batch", 400_000) as usize;
            let skip = flag_u64(args, "--mainnet-skip-blocks", 0) as usize;
            // Execution reads in the file are replayed unless switched off.
            let with_reads = !has(args, "--mainnet-no-reads");
            // Replay one subtree in 2^bits of the key space and scale up.
            let sample_bits = flag_u64(args, "--mainnet-sample-bits", 0) as u32;
            let sample_part = flag_u64(args, "--mainnet-sample-part", 0) as u8;
            let loaded = mainnet::load(
                &diffs,
                &slot_counts,
                scale,
                batch,
                skip,
                with_reads,
                sample_bits,
                sample_part,
            );
            eprintln!("[mainnet] loaded, {}", alloc::heap());
            let blocks = cfg.params.blocks;
            assert!(
                blocks + cfg.warmup_blocks <= loaded.facts.blocks,
                "--blocks plus --warmup-blocks exceeds the {} blocks in the diffs",
                loaded.facts.blocks
            );
            let cfg = bench::BenchConfig {
                cache_sweep: true,
                with_kv: false,
                // The diffs hold writes only. Re-reading a block's own
                // writes would say nothing about mainnet's reads.
                reads_per_block: 0,
                mainnet: Some(bench::MainnetCtx {
                    shape: loaded.shape,
                    scale,
                    facts: loaded.facts,
                }),
                ..cfg
            };
            bench::run_with(cfg, blocks, Box::new(loaded.replay))
        }
    };
    fs::create_dir_all(&out_dir).unwrap();
    fs::write(format!("{out_dir}/{label}.md"), &out.md).unwrap();
    eprintln!("[report] wrote {out_dir}/{label}.md");
    if let Some(csv) = &out.mrc_csv {
        fs::write(format!("{out_dir}/{label}_mrc.csv"), csv).unwrap();
        eprintln!("[report] wrote {out_dir}/{label}_mrc.csv");
    }
    println!("{}", out.md);
}

/// Presence of a valueless flag.
fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn flag_f64(args: &[String], name: &str, default: f64) -> f64 {
    flag(args, name)
        .map(|v| v.parse().expect(name))
        .unwrap_or(default)
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .map(|i| args[i + 1].clone())
}

fn flag_u64(args: &[String], name: &str, default: u64) -> u64 {
    flag(args, name)
        .map(|v| v.parse().expect(name))
        .unwrap_or(default)
}

fn synth(args: &[String]) {
    let mut p = Params::default();
    p.seed = flag_u64(args, "--seed", p.seed);
    p.eoas = flag_u64(args, "--eoas", p.eoas as u64) as usize;
    p.contracts = flag_u64(args, "--contracts", p.contracts as u64) as usize;
    p.templates = flag_u64(args, "--templates", p.templates as u64) as usize;
    p.blocks = flag_u64(args, "--blocks", p.blocks as u64) as usize;
    p.ops_per_block = flag_u64(args, "--ops-per-block", p.ops_per_block as u64) as usize;
    p.mega_contracts = flag_u64(args, "--mega-contracts", p.mega_contracts as u64) as usize;
    p.same_value_pct = flag_u64(args, "--same-value-pct", p.same_value_pct);
    let out_dir = flag(args, "--out").unwrap_or_else(|| "results".into());
    let trace_out = flag(args, "--trace-out");

    let params_dump = format!(
        "seed={} eoas={} contracts={} templates={} blocks={} ops_per_block={} mega_contracts={} same_value_pct={}\nmix: transfer={} call={} churn={} deploy={} selfdestruct={} zipf_s={}",
        p.seed, p.eoas, p.contracts, p.templates, p.blocks, p.ops_per_block, p.mega_contracts, p.same_value_pct,
        p.mix_transfer, p.mix_call, p.mix_churn, p.mix_deploy, p.mix_selfdestruct, p.zipf_s
    );

    let mut world = World::new(p);
    let t0 = Instant::now();
    eprintln!("[gen] building genesis...");
    let genesis = world.genesis();
    eprintln!(
        "[gen] genesis writes: {} ({:.1?})",
        genesis.len(),
        t0.elapsed()
    );

    let blocks = world.params.blocks;
    let mut trace: Option<BufWriter<fs::File>> = trace_out.map(|path| {
        let mut w = BufWriter::new(fs::File::create(path).expect("trace file"));
        writeln!(w, "# pbt-char trace v1").unwrap();
        w
    });

    let mut engine = Engine::new();
    engine.apply_block(&genesis, true);
    if let Some(w) = trace.as_mut() {
        dump_block(w, &genesis);
    }
    eprintln!(
        "[engine] genesis applied: {} entries, root {} ({:.1?})",
        engine.genesis_entries,
        hex(&engine.last_root),
        t0.elapsed()
    );

    let tb = Instant::now();
    for i in 0..blocks {
        let writes = world.block();
        engine.apply_block(&writes, false);
        if let Some(w) = trace.as_mut() {
            dump_block(w, &writes);
        }
        if (i + 1) % 200 == 0 {
            eprintln!("[engine] block {}/{} ({:.1?})", i + 1, blocks, tb.elapsed());
        }
    }
    eprintln!("[engine] {} blocks in {:.1?}", blocks, tb.elapsed());

    finish(engine, &out_dir, "synthetic", params_dump);
}

fn replay(args: &[String]) {
    let path = flag(args, "--trace").expect("--trace FILE required");
    let out_dir = flag(args, "--out").unwrap_or_else(|| "results".into());
    let label = flag(args, "--label").unwrap_or_else(|| "replay".into());

    let file = fs::File::open(&path).expect("open trace");
    let reader = std::io::BufReader::new(file);
    let mut engine = Engine::new();
    let mut block: Vec<StateWrite> = Vec::new();
    let mut first = true;
    let mut seen_boundary = false;
    for line in reader.lines() {
        let line = line.unwrap();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line == "b" {
            if seen_boundary {
                // Close the previous section (possibly empty: an empty first
                // section means "no genesis", so measurement starts at once).
                engine.apply_block(&block, first);
                first = false;
                block.clear();
            }
            seen_boundary = true;
            continue;
        }
        let mut parts = line.split_whitespace();
        assert_eq!(parts.next(), Some("w"));
        let key = unhex(parts.next().expect("key"));
        let value: [u8; 32] = unhex(parts.next().expect("value")).try_into().unwrap();
        block.push((key, value));
    }
    if !block.is_empty() {
        engine.apply_block(&block, first);
    }
    finish(engine, &out_dir, &label, format!("trace={path}"));
}

fn dump_block(w: &mut BufWriter<fs::File>, writes: &[StateWrite]) {
    writeln!(w, "b").unwrap();
    for (k, v) in writes {
        writeln!(w, "w {} {}", hex(k), hex(v)).unwrap();
    }
}

struct Engine {
    tree: PbtTree,
    stats: RewriteStats,
    blocks: u64,
    writes_issued: u64,
    writes_changed: u64,
    genesis_entries: u64,
    last_root: [u8; 32],
    genesis_census: Option<pbt_ref::tree::Census>,
}

impl Engine {
    fn new() -> Self {
        Engine {
            tree: PbtTree::new(),
            stats: RewriteStats::default(),
            blocks: 0,
            writes_issued: 0,
            writes_changed: 0,
            genesis_entries: 0,
            last_root: [0; 32],
            genesis_census: None,
        }
    }

    /// Apply one block of writes, then commit (recompute the root) and
    /// absorb the per-block dirty-node counts. The genesis block builds the
    /// initial state and is excluded from rewrite statistics.
    fn apply_block(&mut self, writes: &[StateWrite], genesis: bool) {
        let mut changed = 0u64;
        for (k, v) in writes {
            if self.tree.write(k, v) {
                changed += 1;
            }
        }
        if genesis {
            self.genesis_entries = changed;
            self.last_root = self.tree.root(); // flush caches, uncounted
            self.genesis_census = Some(self.tree.census());
            return;
        }
        self.writes_issued += writes.len() as u64;
        self.writes_changed += changed;
        self.blocks += 1;
        let mut block_stats = RewriteStats::default();
        self.last_root = self.tree.root_with_stats(&mut block_stats);
        self.stats.absorb(&block_stats);
    }
}

fn finish(engine: Engine, out_dir: &str, label: &str, params_dump: String) {
    fs::create_dir_all(out_dir).expect("out dir");
    let end_census = engine.tree.census();
    let start_census = engine.genesis_census.unwrap_or_default();
    let top = engine.tree.top_structure();
    let info = report::RunInfo {
        label: label.into(),
        blocks: engine.blocks,
        writes_issued: engine.writes_issued,
        writes_changed: engine.writes_changed,
        genesis_entries: engine.genesis_entries,
        params_dump,
    };
    let md = report::report_md(&info, &engine.stats, &start_census, &end_census, &top);
    let csv = report::histogram_csv(&engine.stats, &start_census, &end_census, engine.blocks);
    let pfx = report::prefix_csv(&end_census);
    fs::write(format!("{out_dir}/report_{label}.md"), &md).unwrap();
    fs::write(format!("{out_dir}/histogram_{label}.csv"), &csv).unwrap();
    fs::write(format!("{out_dir}/prefix_hist_{label}.csv"), &pfx).unwrap();
    eprintln!("[report] wrote {out_dir}/report_{label}.md");
    println!("{md}");
}

fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
