//! Report generation: markdown summary + CSV histogram.
//!
//! Frequency semantics (post-verification fixes):
//! - "rewrite" = a record whose committed bytes changed (the §3 variable);
//!   creations and deletions are reported separately as churn.
//! - Denominators use the MEAN of the genesis and end-of-run populations,
//!   since the state grows over the run; both populations are printed.
//! - A cell with zero mean population prints "n/a", never a clamped value.

use pbt_ref::tree::{Census, RewriteStats, TopNode, ZoneLabel};
use std::collections::BTreeMap;
use std::fmt::Write as _;

pub struct RunInfo {
    pub label: String,
    pub blocks: u64,
    pub writes_issued: u64,
    pub writes_changed: u64,
    pub genesis_entries: u64,
    pub params_dump: String,
}

fn zone_order(z: &ZoneLabel) -> u8 {
    match z {
        ZoneLabel::Top => 0,
        ZoneLabel::Account => 1,
        ZoneLabel::Code => 2,
        ZoneLabel::Storage => 3,
        ZoneLabel::Other(_) => 4,
    }
}

fn freq(writes: u64, mean_pop: f64, blocks: u64) -> String {
    if mean_pop <= 0.0 {
        if writes > 0 {
            format!("n/a ({writes} writes, 0 pop)")
        } else {
            "-".into()
        }
    } else {
        format!("{:.8}", writes as f64 / mean_pop / blocks.max(1) as f64)
    }
}

fn all_keys(stats: &RewriteStats, start: &Census, end: &Census) -> Vec<(ZoneLabel, u32)> {
    let mut keys: Vec<_> = stats
        .cells
        .keys()
        .chain(start.cells.keys())
        .chain(end.cells.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    keys.sort_by_key(|(z, d)| (zone_order(z), *d));
    keys
}

pub fn histogram_csv(stats: &RewriteStats, start: &Census, end: &Census, blocks: u64) -> String {
    let mut out = String::from(
        "zone,node_depth,branches_start,branches_end,leaves_start,leaves_end,branch_creates,branch_rewrites,branch_deletes,leaf_creates,leaf_rewrites,leaf_deletes,blocks\n",
    );
    for (zone, depth) in all_keys(stats, start, end) {
        let c = stats.cells.get(&(zone, depth)).copied().unwrap_or_default();
        let ps = start.cells.get(&(zone, depth)).copied().unwrap_or_default();
        let pe = end.cells.get(&(zone, depth)).copied().unwrap_or_default();
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{},{},{},{},{},{}",
            zone.name(),
            depth,
            ps.branches,
            pe.branches,
            ps.leaves,
            pe.leaves,
            c.branch_creates,
            c.branch_rewrites,
            c.branch_deletes,
            c.leaf_creates,
            c.leaf_rewrites,
            c.leaf_deletes,
            blocks
        );
    }
    out
}

pub fn prefix_csv(census: &Census) -> String {
    let mut out = String::from("zone,prefix_bits,count\n");
    for ((zone, bits), n) in &census.prefix_hist {
        let _ = writeln!(out, "{},{},{}", zone.name(), bits, n);
    }
    out
}

#[derive(Default)]
struct ZoneAgg {
    branches_start: u64,
    branches_end: u64,
    leaves_start: u64,
    leaves_end: u64,
    branch_creates: u64,
    branch_rewrites: u64,
    branch_deletes: u64,
    leaf_creates: u64,
    leaf_rewrites: u64,
    leaf_deletes: u64,
    prefix_bits_end: u64,
}

pub fn report_md(
    info: &RunInfo,
    stats: &RewriteStats,
    start: &Census,
    end: &Census,
    top: &[TopNode],
) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "# PBT Phase-1 Characterization: {}\n", info.label);
    let _ = writeln!(
        md,
        "> All workload distributions are synthetic ASSUMPTIONS standing in for\n> mainnet replay; see Parameters. The pipeline, not the absolute numbers,\n> is the validated artifact of this run.\n>\n> Metric semantics: a REWRITE is a surviving record whose committed bytes\n> changed at a block commit. Creations and deletions are churn, reported\n> separately: under SS3 of the draft, append-only cost is driven by rewrites\n> alone (creations generate no garbage). Within-block net-zero patterns\n> (create-then-delete, set-then-restore) are not counted as rewrites.\n> Frequencies divide by the mean of genesis and end populations.\n"
    );

    let _ = writeln!(
        md,
        "## Parameters\n\n```\n{}\n```\n",
        info.params_dump.trim_end()
    );
    let _ = writeln!(
        md,
        "- genesis entries: {}\n- blocks: {}\n- writes issued: {} (changed: {}, no-op {:.2}%)\n",
        info.genesis_entries,
        info.blocks,
        info.writes_issued,
        info.writes_changed,
        100.0 * (info.writes_issued - info.writes_changed) as f64
            / info.writes_issued.max(1) as f64
    );

    let _ = writeln!(md, "## Top structure (Open Item O1)\n");
    let _ = writeln!(
        md,
        "Branch nodes above every zone-pure subtree: **{}**\n",
        top.len()
    );
    for t in top {
        let _ = writeln!(
            md,
            "- node_depth {}: prefix {} bits, children = [{} | {}]",
            t.node_depth,
            t.prefix_bits,
            t.left_label.name(),
            t.right_label.name()
        );
    }

    // Zone aggregates.
    let mut zones: BTreeMap<ZoneLabel, ZoneAgg> = BTreeMap::new();
    for ((z, _), c) in &start.cells {
        let e = zones.entry(*z).or_default();
        e.branches_start += c.branches;
        e.leaves_start += c.leaves;
    }
    for ((z, _), c) in &end.cells {
        let e = zones.entry(*z).or_default();
        e.branches_end += c.branches;
        e.leaves_end += c.leaves;
        e.prefix_bits_end += c.prefix_bits;
    }
    for ((z, _), c) in &stats.cells {
        let e = zones.entry(*z).or_default();
        e.branch_creates += c.branch_creates;
        e.branch_rewrites += c.branch_rewrites;
        e.branch_deletes += c.branch_deletes;
        e.leaf_creates += c.leaf_creates;
        e.leaf_rewrites += c.leaf_rewrites;
        e.leaf_deletes += c.leaf_deletes;
    }

    let _ = writeln!(md, "\n## Per-zone summary\n");
    let _ = writeln!(
        md,
        "| zone | branches (start->end) | leaves (start->end) | branch rewrites | leaf REWRITES | leaf creates | leaf deletes | branch rw/node/blk | leaf rw/node/blk | mean prefix bits (end) |"
    );
    let _ = writeln!(md, "|---|---|---|---|---|---|---|---|---|---|");
    let mut zlist: Vec<_> = zones.iter().collect();
    zlist.sort_by_key(|(z, _)| zone_order(z));
    for (z, a) in zlist {
        let mean_b = (a.branches_start + a.branches_end) as f64 / 2.0;
        let mean_l = (a.leaves_start + a.leaves_end) as f64 / 2.0;
        let _ = writeln!(
            md,
            "| {} | {}->{} | {}->{} | {} | {} | {} | {} | {} | {} | {:.1} |",
            z.name(),
            a.branches_start,
            a.branches_end,
            a.leaves_start,
            a.leaves_end,
            a.branch_rewrites,
            a.leaf_rewrites,
            a.leaf_creates,
            a.leaf_deletes,
            freq(a.branch_rewrites, mean_b, info.blocks),
            freq(a.leaf_rewrites, mean_l, info.blocks),
            a.prefix_bits_end as f64 / a.branches_end.max(1) as f64
        );
    }

    let _ = writeln!(
        md,
        "\n## Leaf placement depth (bits consumed to leaf position, end)\n"
    );
    let _ = writeln!(md, "| zone | leaves | mean bit-depth |");
    let _ = writeln!(md, "|---|---|---|");
    for (z, (sum, count)) in &end.leaf_bit_depth_sum {
        let _ = writeln!(
            md,
            "| {} | {} | {:.1} |",
            z.name(),
            count,
            *sum as f64 / (*count).max(1) as f64
        );
    }

    let _ = writeln!(md, "\n## Rewrite frequency by zone x node depth\n");
    let _ = writeln!(
        md,
        "Frequencies use REWRITES only (creations/deletions shown as churn).\nHigh rewrite frequency indicates page-packed or DRAM-resident; low favors\nappend-only (draft v8 SS3). Populations shown as start->end; the frequency\ndenominator is their mean.\n"
    );
    let _ = writeln!(
        md,
        "| zone | depth | branches | br rewrites | br rw/node/blk | leaves | leaf rewrites | leaf rw/node/blk | leaf creates | leaf deletes |"
    );
    let _ = writeln!(md, "|---|---|---|---|---|---|---|---|---|---|");
    for (zone, depth) in all_keys(stats, start, end) {
        let c = stats.cells.get(&(zone, depth)).copied().unwrap_or_default();
        let ps = start.cells.get(&(zone, depth)).copied().unwrap_or_default();
        let pe = end.cells.get(&(zone, depth)).copied().unwrap_or_default();
        let mean_b = (ps.branches + pe.branches) as f64 / 2.0;
        let mean_l = (ps.leaves + pe.leaves) as f64 / 2.0;
        let _ = writeln!(
            md,
            "| {} | {} | {}->{} | {} | {} | {}->{} | {} | {} | {} | {} |",
            zone.name(),
            depth,
            ps.branches,
            pe.branches,
            c.branch_rewrites,
            freq(c.branch_rewrites, mean_b, info.blocks),
            ps.leaves,
            pe.leaves,
            c.leaf_rewrites,
            freq(c.leaf_rewrites, mean_l, info.blocks),
            c.leaf_creates,
            c.leaf_deletes
        );
    }

    let _ = writeln!(md, "\nMax node depth (end): {}\n", end.max_node_depth);
    md
}
