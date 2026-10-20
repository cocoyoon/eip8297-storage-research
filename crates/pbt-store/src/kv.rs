//! Mutable key-value control: real libmdbx. This is the reference point
//! every other backend is measured against, and the only backend whose
//! read path is fully real (page cache, B-tree descent, MVCC included).
//!
//! Records map to KV entries the way a client would do it today: one entry
//! per node record, keyed by its stable tree position (leaves by their
//! complete key, branches by their packed bit path).

use crate::{Backend, Metrics};
use libmdbx::{
    Database, DatabaseOptions, Mode, PageSize, ReadWriteOptions, SyncMode, Table, TableFlags,
    Transaction, TransactionKind, WriteFlags, WriteMap,
};
use pbt_ref::tree::{CommitSink, ReadPath, RecordKind};
use std::path::Path;
use std::time::Instant;

pub struct KvBackend {
    env: Database<WriteMap>,
    m: Metrics,
    dir: String,
}

impl KvBackend {
    pub fn new(dir: &str) -> Self {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).expect("kv dir");
        let opts = DatabaseOptions {
            max_tables: Some(4),
            page_size: Some(PageSize::Set(4096)),
            mode: Mode::ReadWrite(ReadWriteOptions {
                // Durable: one fsync per block commit, matching the other
                // backends' durability point.
                sync_mode: SyncMode::Durable,
                min_size: Some(1 << 30),
                max_size: Some(96 << 30),
                growth_step: Some(1 << 30),
                shrink_threshold: None,
            }),
            ..Default::default()
        };
        let env = Database::<WriteMap>::open_with_options(Path::new(dir), opts).expect("open mdbx");
        {
            let tx = env.begin_rw_txn().unwrap();
            tx.create_table(Some("nodes"), TableFlags::empty()).unwrap();
            tx.commit().unwrap();
        }
        KvBackend {
            env,
            m: Metrics::default(),
            dir: dir.into(),
        }
    }

    fn table<'t, K: TransactionKind>(tx: &'t Transaction<'t, K, WriteMap>) -> Table<'t> {
        tx.open_table(Some("nodes")).unwrap()
    }

    fn id(kind: RecordKind, pos: &[u8], leaf_key: Option<&[u8]>) -> Vec<u8> {
        match kind {
            RecordKind::Leaf => {
                let mut v = Vec::with_capacity(1 + leaf_key.unwrap().len());
                v.push(0u8);
                v.extend_from_slice(leaf_key.unwrap());
                v
            }
            RecordKind::Branch => {
                let mut v = Vec::with_capacity(1 + pos.len());
                v.push(1u8);
                v.extend_from_slice(pos);
                v
            }
        }
    }
}

impl Backend for KvBackend {
    fn name(&self) -> String {
        "kv-mdbx".into()
    }

    fn commit_block(&mut self, batch: &CommitSink) {
        let t0 = Instant::now();
        let tx = self.env.begin_rw_txn().unwrap();
        {
            let db = Self::table(&tx);
            let mut payload = Vec::new();
            for r in &batch.records {
                let k = Self::id(r.kind, &r.pos, r.leaf_key.as_deref());
                payload.clear();
                payload.resize(r.bytes as usize, 0xEE);
                tx.put(&db, &k, &payload, WriteFlags::UPSERT).unwrap();
                self.m.record_writes += 1;
            }
            for d in &batch.deletes {
                let kind = if d.was_leaf {
                    RecordKind::Leaf
                } else {
                    RecordKind::Branch
                };
                let k = Self::id(kind, &d.pos, d.leaf_key.as_deref());
                let _ = tx.del(&db, &k, None);
                self.m.record_deletes += 1;
            }
        }
        tx.commit().unwrap();
        self.m.fsyncs += 1;
        self.m.wall_ns += t0.elapsed().as_nanos() as u64;
    }

    fn serve_read(&mut self, rp: &ReadPath) {
        self.m.reads_served += 1;
        let tx = self.env.begin_ro_txn().unwrap();
        let db = Self::table(&tx);
        for (_sb, pos, _np, _b) in &rp.branches {
            let k = Self::id(RecordKind::Branch, pos, None);
            let _: Option<Vec<u8>> = tx.get(&db, &k).unwrap();
            self.m.page_reads += 1;
            self.m.read_bytes += 4096;
        }
        if let Some((lk, _)) = &rp.leaf {
            let k = Self::id(RecordKind::Leaf, &[], Some(lk));
            let _: Option<Vec<u8>> = tx.get(&db, &k).unwrap();
            self.m.page_reads += 1;
            self.m.read_bytes += 4096;
        }
    }

    fn serve_value_read(&mut self, rp: &ReadPath) {
        self.m.reads_served += 1;
        let tx = self.env.begin_ro_txn().unwrap();
        let db = Self::table(&tx);
        if let Some((lk, _)) = &rp.leaf {
            let k = Self::id(RecordKind::Leaf, &[], Some(lk));
            let _: Option<Vec<u8>> = tx.get(&db, &k).unwrap();
            self.m.page_reads += 1;
            self.m.read_bytes += 4096;
        }
    }

    fn metrics(&self) -> Metrics {
        let mut m = self.m.clone();
        m.disk_bytes = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter_map(|e| e.metadata().ok())
                    .map(|md| md.len())
                    .sum()
            })
            .unwrap_or(0);
        m
    }

    fn finalize(&mut self) {
        self.env.sync(true).ok();
    }

    fn extra_report(&self) -> String {
        let st = self.env.stat().ok();
        match st {
            Some(s) => format!(
                "- kv-mdbx: depth {}, branch pages {}, leaf pages {}, overflow pages {}, entries {}\n",
                s.depth(),
                s.branch_pages(),
                s.leaf_pages(),
                s.overflow_pages(),
                s.entries()
            ),
            None => String::new(),
        }
    }
}
