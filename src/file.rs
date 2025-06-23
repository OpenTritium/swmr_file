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

pub(crate) type WritedRange = Arc<RwLock<RangeSetBlaze<u64>>>;
pub(crate) type SyncedRange = Arc<Mutex<RangeSetBlaze<u64>>>;

pub struct WriteReadFile {
    path: Utf8PathBuf,
    writed: WritedRange,
    flushed: SyncedRange, // 在停机时及时保存状态，持久化到硬盘
}

impl WriteReadFile {
    pub fn new(path: Utf8PathBuf) -> Self {
        Self {
            path,
            writed: Arc::new(RwLock::new(RangeSetBlaze::new())),
            flushed: Arc::new(Mutex::new(RangeSetBlaze::new())),
        }
    }
}

struct ReadOnlyFile {
    path: Utf8PathBuf,
}

pub trait Writable<S: SyncStrategy> {
    fn get_writer(&self, strategy: S) -> StateWriter<S>;
}

pub trait Readable {
    fn get_reader(&self) -> impl AsyncRead;
}

impl<S: SyncStrategy> Writable<S> for WriteReadFile {
    fn get_writer(&self, strategy: S) -> StateWriter<S> {
        let fd = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&self.path)
            .unwrap();
        let fd = SyncFile::from(fd);
        StateWriter::new(fd, strategy, self.writed.clone(), self.flushed.clone())
    }
}

impl Readable for WriteReadFile {
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
