use sync_file::SyncFile;
use tokio::io::AsyncRead;
use crate::file::WritedRange;

pub struct StateReader {
    writed: WritedRange,
    fd: SyncFile,
}

impl StateReader {
    pub fn new(writed: WritedRange, fd: SyncFile) -> Self {
        Self { writed, fd }
    }
}

impl AsyncRead for StateReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        todo!()
    }
}

pub struct Reader {
    fd: SyncFile,
}

impl Reader {
    pub fn new(fd: SyncFile) -> Self {
        Self { fd }
    }
}

impl AsyncRead for Reader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        todo!()
    }
}
