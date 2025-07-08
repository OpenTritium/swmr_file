use sync_file::SyncFile;
use tokio::io::AsyncRead;

pub struct Reader {
    file: SyncFile,
}

impl Reader {
    pub fn new(file: SyncFile) -> Self {
        Self { file }
    }
}

impl AsyncRead for Reader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        dst: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        todo!()
    }
}
