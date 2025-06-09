use crate::{
    reader::{Reader, StateReader},
    strategy::SyncStrategy,
    writer::StateWriter,
};
use camino::Utf8PathBuf;
use range_set_blaze::RangeSetBlaze;
use std::{fs::OpenOptions, sync::Arc};
use sync_file::SyncFile;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{Mutex, RwLock},
};

pub(crate) type WritedRange = Arc<RwLock<RangeSetBlaze<usize>>>;
pub(crate) type SyncedRange = Arc<Mutex<RangeSetBlaze<usize>>>;

struct ReadWriteFile {
    path: Utf8PathBuf,
    writed: WritedRange,  // 存在并发读
    flushed: SyncedRange, // 只用于持久化
}

struct ReadOnlyFile {
    path: Utf8PathBuf,
}

trait Writable {
    fn get_writer(&self, strategy: impl SyncStrategy) -> impl AsyncWrite;
}

trait Readable {
    fn get_reader(&self) -> impl AsyncRead;
}

impl Writable for ReadWriteFile {
    fn get_writer(&self, strategy: impl SyncStrategy) -> impl AsyncWrite {
        let fd = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&self.path)
            .unwrap();
        let fd = SyncFile::from(fd);
        StateWriter::new(fd, strategy, self.writed.clone(), self.flushed.clone())
    }
}

impl Readable for ReadWriteFile {
    fn get_reader(&self) -> impl AsyncRead {
        let fd = SyncFile::open(&self.path).unwrap();
        StateReader::new(self.writed.clone(), fd)
    }
}

impl Readable for ReadOnlyFile {
    fn get_reader(&self) -> impl AsyncRead {
        let fd = SyncFile::open(&self.path).unwrap();
        Reader::new(fd)
    }
}
