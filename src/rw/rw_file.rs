use camino::Utf8PathBuf;
use range_set_blaze::RangeSetBlaze;
use std::{fs::OpenOptions, io::Result as IoResult, sync::Arc};
use sync_file::SyncFile;
use tokio::sync::{Mutex, RwLock};

pub(crate) type WritedRange = Arc<RwLock<RangeSetBlaze<u64>>>;
pub(crate) type SyncedRange = Arc<Mutex<RangeSetBlaze<u64>>>;

pub struct RwFile {
    path: Utf8PathBuf,
    file: SyncFile,
    writed: WritedRange,
    flushed: SyncedRange, // 在停机时及时保存状态，持久化到硬盘
}

impl RwFile {
    /// 以读写模式打开某个文件，若文件不存在则创建，存在则截断
    pub fn create(path: Utf8PathBuf) -> IoResult<Self> {
        // todo! 实现一个协程定时同步range
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path.as_path())?;
        Ok(Self {
            path,
            file: SyncFile::from(file),
            writed: Arc::new(RwLock::new(RangeSetBlaze::new())),
            flushed: Arc::new(Mutex::new(RangeSetBlaze::new())),
        })
    }

    /// 以只读模式打开某个文件
    pub fn open(path: Utf8PathBuf) -> IoResult<Self> {
        // todo! 实现一个协程定时同步range
        Ok(Self {
            file: SyncFile::open(path.as_path())?,
            path,
            writed: Arc::new(RwLock::new(RangeSetBlaze::new())),
            flushed: Arc::new(Mutex::new(RangeSetBlaze::new())),
        })
    }

    pub fn proceed(path: Utf8PathBuf) -> IoResult<Self> {
        //  具体实现为根据文件的拓展属性恢复进度
        todo!()
    }
}
