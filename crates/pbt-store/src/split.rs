//! MEASUREMENT CONTROL, not a layout candidate. Retained after the tier
//! decision (draft Appendix D) because it demonstrates that composing a
//! page tier above a log tier costs ~115 lines behind an unchanged
//! `Backend` trait, and because Appendix D's reinstatement trigger runs
//! against it. Note the cross-store hazard it exercises and that the
//! committed layout no longer has: an elided page is rebuilt from the leaf
//! store, so log compaction must never reclaim a record a live elided page
//! still needs. That constraint is unimplemented.
//!
//! Depth-split backend: page-packed above a cut depth, append-only below.
//! Sweeping the cut is how Phase 2 locates `d*` (RQ1) with the commitment
//! held fixed and everything else about the two disciplines unchanged.

use crate::append::{AppendCore, BranchHashPolicy};
use crate::paged::{Packing, PagedCore};
use crate::{Backend, Metrics};
use pbt_ref::tree::{CommitSink, ReadPath};
use std::time::Instant;

pub struct SplitBackend {
    cut: u32,
    paged: PagedCore,
    append: AppendCore,
    wall_ns: u64,
}

impl SplitBackend {
    /// Records with node_depth < cut go to the paged store; the rest append.
    pub fn new(cut: u32, resident_levels: u32) -> Self {
        SplitBackend {
            cut,
            paged: PagedCore::with_packing(resident_levels, Packing::Structural),
            // Identical DRAM grant on both sides of the cut.
            // Same branch-hash policy the standalone log store uses, so the
            // cut sweep varies only the tier boundary (see append.rs).
            append: AppendCore::with_policy(resident_levels * 6, BranchHashPolicy::CacheInDram),
            wall_ns: 0,
        }
    }
}

impl Backend for SplitBackend {
    fn name(&self) -> String {
        format!("split(cut={})", self.cut)
    }

    fn commit_block(&mut self, batch: &CommitSink) {
        let t0 = Instant::now();
        let cut = self.cut;
        self.paged.apply(batch, |d| d < cut);
        self.append.apply(batch, |d| d >= cut);
        self.append.maintain();
        // One durability point for the whole commit: WAL + manifest + pages,
        // plus the log's own fsync (SS6.5's cross-backend atomicity cost).
        self.paged.m.fsyncs += 3;
        self.append.m.fsyncs += 1;
        self.wall_ns += t0.elapsed().as_nanos() as u64;
    }

    fn serve_read(&mut self, rp: &ReadPath) {
        let cut = self.cut;
        self.paged.read(rp, |d| d < cut);
        self.append.read(rp, |d| d >= cut);
        self.paged.m.reads_served += 1;
    }

    fn serve_read_batch(&mut self, rps: &[ReadPath]) {
        let cut = self.cut;
        // One multiproof spanning both tiers: pages above the cut and
        // records below it are each fetched once for the whole batch.
        self.paged.read_batch(rps, |d| d < cut);
        self.append.read_batch(rps, |d| d >= cut);
        self.paged.m.reads_served += rps.len() as u64;
    }

    fn serve_value_read(&mut self, rp: &ReadPath) {
        // Values live in the append tier below the cut; the index resolves
        // them directly, exactly as in the pure append design.
        self.paged.m.reads_served += 1;
        if let Some((_k, bytes)) = &rp.leaf {
            self.append.m.page_reads += 1;
            self.append.m.read_bytes += (*bytes as u64).div_ceil(512) * 512;
        }
    }

    fn metrics(&self) -> Metrics {
        let a = &self.append.m;
        let p = &self.paged.m;
        Metrics {
            seq_bytes_written: a.seq_bytes_written + p.seq_bytes_written,
            seq_bytes_read: a.seq_bytes_read + p.seq_bytes_read,
            rand_page_writes: a.rand_page_writes + p.rand_page_writes,
            page_reads: a.page_reads + p.page_reads,
            read_bytes: a.read_bytes + p.read_bytes,
            fsyncs: a.fsyncs + p.fsyncs,
            live_bytes: a.live_bytes + p.live_bytes,
            garbage_bytes: a.garbage_bytes,
            compactions: a.compactions,
            record_writes: a.record_writes + p.record_writes,
            record_deletes: a.record_deletes + p.record_deletes,
            reads_served: p.reads_served,
            wall_ns: self.wall_ns,
            disk_bytes: a.live_bytes + self.paged.page_count() as u64 * crate::PAGE,
            // The append side needs its index resident; QMDB's measured
            // compact index is ~15.4 B/key in-memory.
            dram_bytes: self.paged.dram_bytes() + self.append.dram_bytes(),
        }
    }

    fn finalize(&mut self) {
        self.append.settle();
        self.paged.sample_occupancy();
    }

    fn extra_report(&self) -> String {
        format!(
            "- split(cut={}): paged {} pages / {} rand-writes; append live {} MB garbage {} MB compactions {}; append records {} branches {}; dram bytes {} (paged {} + append {})\n",
            self.cut,
            self.paged.page_count(),
            self.paged.m.rand_page_writes,
            self.append.m.live_bytes >> 20,
            self.append.m.garbage_bytes >> 20,
            self.append.m.compactions,
            self.append.index.len(),
            self.append.live_branch_count(),
            self.paged.dram_bytes() + self.append.dram_bytes(),
            self.paged.dram_bytes(),
            self.append.dram_bytes()
        )
    }
}
