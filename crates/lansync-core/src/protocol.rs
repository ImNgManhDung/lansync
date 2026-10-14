//! Protocol messages between peers and frame packaging.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTO_VERSION: u16 = 1;
pub const MAX_FRAME: u32 = 16 * 1024 * 1024;
pub const CHUNK_SIZE: usize = 256 * 1024;
pub const INDEX_BATCH: usize = 5_000;

pub type Hash = [u8; 32];

/// A file entry in the index. `hash = None` means the file was deleted locally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub hash: Option<Hash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    Hello { device_id: String, name: String, proto: u16, pairing: bool, listen_port: u16 },
    PairDecision { accept: bool },
    Index { entries: Vec<Entry> },
    IndexUpdate { entries: Vec<Entry> },
    GetFile { path: String, hash: Hash },
    FileHeader { path: String, size: u64, hash: Hash, mtime_ns: i64 },
    FileChunk { data: Vec<u8> },
    FileEnd,
    FileUnavailable { path: String },
    Ping,
    Pong,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    fn entry(n: usize) -> Entry {
        Entry { path: format!("f{n}.txt"), size: n as u64, mtime_ns: 1, hash: Some([n as u8; 32]) }
    }

    #[tokio::test]
    async fn frame_roundtrip() {
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        let msgs = vec![
            Message::Hello { device_id: "ID".into(), name: "Mac".into(), proto: PROTO_VERSION, pairing: false, listen_port: 47800 },
            Message::Index { entries: vec![entry(1), Entry { hash: None, ..entry(2) }] },
            Message::FileChunk { data: vec![7; 1000] },
            Message::Ping,
        ];
        for m in &msgs {
            write_msg(&mut a, m).await.unwrap();
        }
        for m in &msgs {
            assert_eq!(&read_msg(&mut b).await.unwrap(), m);
        }
    }

    #[tokio::test]
    async fn rejects_oversized_frame() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        a.write_u32(MAX_FRAME + 1).await.unwrap();
        let err = read_msg(&mut b).await.unwrap_err();
        assert!(err.to_string().contains("frame too large"), "{err}");
    }

    #[test]
    fn index_messages_batches() {
        let entries: Vec<Entry> = (0..12_001).map(entry).collect();
        let msgs = index_messages(entries, true);
        assert_eq!(msgs.len(), 3);
        assert!(matches!(&msgs[0], Message::Index { entries } if entries.len() == INDEX_BATCH));
        assert!(matches!(&msgs[2], Message::IndexUpdate { entries } if entries.len() == 2_001));
        // Empty index must still send 1 empty Index frame so remote knows scan completed
        assert_eq!(index_messages(vec![], true), vec![Message::Index { entries: vec![] }]);
        assert!(index_messages(vec![], false).is_empty());
    }
}
