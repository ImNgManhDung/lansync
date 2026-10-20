//! Protocol messages between peers and frame packaging.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTO_VERSION: u16 = 1;
pub const MAX_FRAME: u32 = 16 * 1024 * 1024;
pub const CHUNK_SIZE: usize = 256 * 1024;
pub const INDEX_BATCH: usize = 5_000;

pub type Hash = [u8; 32];

use crate::version_vector::VersionVector;

/// A file entry in the index. `hash = None` means the file was deleted locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub hash: Option<Hash>,
    #[serde(default)]
    pub vv: VersionVector,
}

impl Entry {
    pub fn simple(path: impl Into<String>, size: u64, mtime_ns: i64, hash: Option<Hash>) -> Self {
        Self {
            path: path.into(),
            size,
            mtime_ns,
            hash,
            vv: VersionVector::default(),
        }
    }

    pub fn with_vv(mut self, vv: VersionVector) -> Self {
        self.vv = vv;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    Hello { device_id: String, name: String, proto: u16, pairing: bool, listen_port: u16 },
    PairDecision { accept: bool },
    Index { entries: Vec<Entry> },
    IndexUpdate { entries: Vec<Entry> },
    GetFile { path: String, hash: Hash },
    GetFileRange { path: String, hash: Hash, offset: u64 },
    FileHeader { path: String, size: u64, hash: Hash, mtime_ns: i64 },
    FileHeaderRange { path: String, size: u64, hash: Hash, mtime_ns: i64, offset: u64 },
    FileChunk { data: Vec<u8> },
    FileEnd,
    FileUnavailable { path: String },
    Ping,
    Pong,
    Unpair,
}

pub async fn write_msg<W: AsyncWrite + Unpin>(w: &mut W, msg: &Message) -> Result<()> {
    let buf = bincode::serialize(msg)?;
    ensure!(buf.len() as u64 <= MAX_FRAME as u64, "frame too large: {} bytes", buf.len());
    w.write_u32(buf.len() as u32).await?;
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_msg<R: AsyncRead + Unpin>(r: &mut R) -> Result<Message> {
    let len = r.read_u32().await?;
    ensure!(len <= MAX_FRAME, "frame too large: {len} bytes");
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(bincode::deserialize(&buf)?)
}

/// Splits index into frames. `full = true`: first frame is `Index` (full replacement),
/// subsequent frames are `IndexUpdate`.
pub fn index_messages(entries: Vec<Entry>, full: bool) -> Vec<Message> {
    let mut chunks = entries.chunks(INDEX_BATCH).map(|c| c.to_vec());
    let mut out = Vec::new();
    if full {
        out.push(Message::Index { entries: chunks.next().unwrap_or_default() });
    }
    out.extend(chunks.map(|entries| Message::IndexUpdate { entries }));
    out
}

