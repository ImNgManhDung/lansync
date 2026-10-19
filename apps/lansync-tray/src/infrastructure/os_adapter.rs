use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

use crate::domain::ports::SystemServicePort;

#[derive(Default)]
pub struct OsSystemAdapter;

impl OsSystemAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SystemServicePort for OsSystemAdapter {
    fn local_lan_ip(&self) -> Option<String> {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        socket.connect("8.8.8.8:80").ok()?;
        Some(socket.local_addr().ok()?.ip().to_string())
    }

    fn open_folder(&self, path: &Path) -> Result<()> {
        open::that(path).map_err(|e| anyhow!("Không thể mở thư mục: {}", e))
    }

    async fn pick_folder(&self) -> Option<PathBuf> {
        let folder = rfd::AsyncFileDialog::new()
            .set_title("Chọn thư mục đồng bộ LanSync")
            .pick_folder()
            .await;
        folder.map(|f| f.path().to_path_buf())
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<()> {
        let mut cb = arboard::Clipboard::new()
            .map_err(|e| anyhow!("Không thể truy cập clipboard: {}", e))?;
        cb.set_text(text.to_string())
            .map_err(|e| anyhow!("Không thể sao chép vào clipboard: {}", e))
    }
}
