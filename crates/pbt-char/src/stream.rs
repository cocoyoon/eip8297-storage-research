//! A record-update stream exported from a replay, so that a real engine
//! can be fed the same updates the report-time models were: every record
//! a block commits (its tree position and serialized size) and every
//! delete, in order, the starting state first.
//!
//! Format `pbtstream1`, little-endian: `G` opens the starting state, `S`
//! the replayed blocks, `B u32` one block. `C`/`W` u8-length key u16-bytes
//! is a created/rewritten record; `D` u8-length key is a delete. The key is
//! the record's position (`encode_bit_prefix`: a big-endian bit count, then
//! the bits), the same for leaves and branches.

use pbt_ref::tree::{CommitSink, RecordOp};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};

pub const MAGIC: &[u8; 11] = b"pbtstream1\n";

pub struct StreamWriter {
    w: BufWriter<File>,
}

impl StreamWriter {
    pub fn create(path: &str) -> io::Result<Self> {
        let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
        w.write_all(MAGIC)?;
        Ok(StreamWriter { w })
    }

    pub fn genesis(&mut self) -> io::Result<()> {
        self.w.write_all(b"G")
    }

    pub fn start(&mut self) -> io::Result<()> {
        self.w.write_all(b"S")
    }

    pub fn block(&mut self, i: u32) -> io::Result<()> {
        self.w.write_all(b"B")?;
        self.w.write_all(&i.to_le_bytes())
    }

    /// Every record and delete of one commit, records first, as the
    /// backends see them.
    pub fn sink(&mut self, sink: &CommitSink) -> io::Result<()> {
        for r in &sink.records {
            self.w
                .write_all(if r.op == RecordOp::Create { b"C" } else { b"W" })?;
            self.key(&r.pos)?;
            let bytes = r.bytes.min(u16::MAX as u32) as u16;
            self.w.write_all(&bytes.to_le_bytes())?;
        }
        for d in &sink.deletes {
            self.w.write_all(b"D")?;
            self.key(&d.pos)?;
        }
        Ok(())
    }

    fn key(&mut self, k: &[u8]) -> io::Result<()> {
        assert!(k.len() <= u8::MAX as usize, "key of {} bytes", k.len());
        self.w.write_all(&[k.len() as u8])?;
        self.w.write_all(k)
    }

    pub fn finish(mut self) -> io::Result<()> {
        self.w.flush()
    }
}

#[cfg_attr(not(feature = "rocksdb"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Genesis,
    Start,
    Block(u32),
    /// A record written; its key is `StreamReader::key`.
    Put {
        create: bool,
        bytes: u32,
    },
    /// A record removed; its key is `StreamReader::key`.
    Delete,
}

#[cfg_attr(not(feature = "rocksdb"), allow(dead_code))]
pub struct StreamReader {
    r: BufReader<File>,
    key: Vec<u8>,
}

#[cfg_attr(not(feature = "rocksdb"), allow(dead_code))]
impl StreamReader {
    pub fn open(path: &str) -> io::Result<Self> {
        let mut r = BufReader::with_capacity(1 << 20, File::open(path)?);
        let mut m = [0u8; 11];
        r.read_exact(&mut m)?;
        if &m != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a pbtstream1 file",
            ));
        }
        Ok(StreamReader { r, key: Vec::new() })
    }

    /// The key of the last `Put` or `Delete`.
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    pub fn next(&mut self) -> io::Result<Option<Event>> {
        let mut tag = [0u8; 1];
        match self.r.read_exact(&mut tag) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        Ok(Some(match tag[0] {
            b'G' => Event::Genesis,
            b'S' => Event::Start,
            b'B' => {
                let mut b = [0u8; 4];
                self.r.read_exact(&mut b)?;
                Event::Block(u32::from_le_bytes(b))
            }
            t @ (b'C' | b'W') => {
                self.read_key()?;
                let mut b = [0u8; 2];
                self.r.read_exact(&mut b)?;
                Event::Put {
                    create: t == b'C',
                    bytes: u16::from_le_bytes(b) as u32,
                }
            }
            b'D' => {
                self.read_key()?;
                Event::Delete
            }
            t => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("bad tag {t}"),
                ))
            }
        }))
    }

    fn read_key(&mut self) -> io::Result<()> {
        let mut l = [0u8; 1];
        self.r.read_exact(&mut l)?;
        self.key.resize(l[0] as usize, 0);
        self.r.read_exact(&mut self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbt_ref::tree::{CommitRecord, DeleteEvent, RecordKind, ZoneLabel};

    #[test]
    fn a_stream_reads_back_what_was_written() {
        let path = std::env::temp_dir().join(format!("pbtstream-{}.bin", std::process::id()));
        let path = path.to_str().unwrap().to_string();
        let mut sink = CommitSink::default();
        sink.records.push(CommitRecord {
            kind: RecordKind::Branch,
            op: RecordOp::Rewrite,
            zone: ZoneLabel::Account,
            node_depth: 3,
            start_bit: 3,
            pos: vec![0, 3, 0xA0],
            node_pos: vec![],
            leaf_key: None,
            bytes: 70,
        });
        sink.records.push(CommitRecord {
            kind: RecordKind::Leaf,
            op: RecordOp::Create,
            zone: ZoneLabel::Storage,
            node_depth: 9,
            start_bit: 9,
            pos: vec![0, 9, 0xA5, 0x80],
            node_pos: vec![],
            leaf_key: Some(vec![1u8; 33].into_boxed_slice()),
            bytes: 66,
        });
        sink.deletes.push(DeleteEvent {
            zone_byte: 0,
            node_depth: 9,
            was_leaf: true,
            start_bit: 9,
            pos: vec![0, 9, 0xA5, 0x00],
            node_pos: vec![],
            leaf_key: None,
            bytes: 66,
        });
        let mut w = StreamWriter::create(&path).unwrap();
        w.genesis().unwrap();
        w.sink(&sink).unwrap();
        w.start().unwrap();
        w.block(7).unwrap();
        w.sink(&sink).unwrap();
        w.finish().unwrap();

        let mut r = StreamReader::open(&path).unwrap();
        assert_eq!(r.next().unwrap(), Some(Event::Genesis));
        assert_eq!(
            r.next().unwrap(),
            Some(Event::Put {
                create: false,
                bytes: 70
            })
        );
        assert_eq!(r.key(), &[0, 3, 0xA0]);
        assert_eq!(
            r.next().unwrap(),
            Some(Event::Put {
                create: true,
                bytes: 66
            })
        );
        assert_eq!(r.key(), &[0, 9, 0xA5, 0x80]);
        assert_eq!(r.next().unwrap(), Some(Event::Delete));
        assert_eq!(r.key(), &[0, 9, 0xA5, 0x00]);
        assert_eq!(r.next().unwrap(), Some(Event::Start));
        assert_eq!(r.next().unwrap(), Some(Event::Block(7)));
        for _ in 0..3 {
            assert!(r.next().unwrap().is_some());
        }
        assert_eq!(r.next().unwrap(), None);
        std::fs::remove_file(&path).unwrap();
    }
}
