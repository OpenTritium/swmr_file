use std::io::{ErrorKind as IoErrorKind, Read, Result as IoResult, Seek};
use sync_file::SyncFile;
use tokio::io::ReadBuf;

pub(crate) type RingBuffer = slice_ring_buffer::SliceRingBuffer<u8>;

pub(crate) trait RingBufferExt {
    fn obsolete(&mut self) -> isize;
    fn read_file(&mut self, src: SyncFile) -> IoResult<usize>;
    fn write_to(&mut self, dst: &mut ReadBuf<'_>) -> usize;
    fn clear_shrink(&mut self, n: usize);
    fn read_from(&mut self, src: &[u8]) -> usize;
}

macro_rules! uninterruptibly {
    ($e:expr) => {{
        loop {
            match $e {
                Err(ref err) if err.kind() == IoErrorKind::Interrupted => {}
                result => break result,
            }
        }
    }};
}

pub(crate) const BUFFER_MIN_SIZE: usize = 4 * 0x400; // 一个簇大小
pub(crate) const BUFFER_MAX_SIZE: usize = 32 * BUFFER_MIN_SIZE;

impl RingBufferExt for RingBuffer {
    #[inline(always)]
    /// 丢弃该缓冲中的所有数据，并返回一个负数以表示丢弃的字节数
    fn obsolete(&mut self) -> isize {
        use std::ops::Neg;
        let n = self.len();
        self.clear();
        isize::try_from(n)
            .expect("obsolete buf count overflowed")
            .neg()
    }

    #[inline(always)]
    /// 此函数确保缓冲区大小在 `[MAX_BUF, src.len()]` 之间，
    /// 必须清空本缓冲区后再调用此函数，
    /// src 带有游标状态，所以每次读取 src 都是后面的新数据，为了连续读取，不要去变更 src 游标
    fn read_file(&mut self, mut src: SyncFile) -> IoResult<usize> {
        debug_assert!(self.is_empty());
        let n = BUFFER_MAX_SIZE.min(src.stream_len()? as usize);
        self.reserve(n);
        let uninit = unsafe {
            let uninit = self.tail_head_slice();
            std::slice::from_raw_parts_mut(uninit.as_mut_ptr() as *mut u8, uninit.len())
        };
        let result = uninterruptibly!(src.read(uninit));
        result
            .inspect(|&n| unsafe {
                self.move_tail_unchecked(
                    n.try_into().expect(
                        "将无符号的文件偏移量 u64 转换到 ringbuf 的有符号偏移量 isize 时失败",
                    ),
                );
            })
            .inspect_err(|_| {
                self.clear_shrink(BUFFER_MIN_SIZE);
            })
    }

    #[inline(always)]
    /// 此函数不影响缓冲区大小(（self 与 dst），
    /// 每次写入量在 `[self.len(), dst.remaining()]` 间，
    /// 你可以持续调用此函数，直到 self 被消耗殆尽
    fn write_to(&mut self, dst: &mut ReadBuf<'_>) -> usize {
        let n = self.len().min(dst.remaining()); // 保证 put_slice 安全
        dst.put_slice(&self[..n]);
        unsafe {
            self.move_head_unchecked(
                n.try_into()
                    .expect("将ringbuf的缓冲拷贝到dst中时，协商字节数量 usize 转换到 isize 时溢出"),
            )
        };
        n
    }

    #[inline(always)]
    /// 清空缓冲区，并保证缓冲区容量不大于 n
    fn clear_shrink(&mut self, n: usize) {
        self.truncate(n);
        self.clear();
    }

    #[inline(always)]
    /// 读取一部分 src，读取的字节数量在 `[src.len(), BUFFER_MAX_SIZE]` 之间，
    /// 调用此函数之间请保证本缓冲区已被清理完毕
    fn read_from(&mut self, src: &[u8]) -> usize {
        debug_assert!(self.is_empty());
        let n = BUFFER_MAX_SIZE.min(src.len());
        self.extend_from_slice(&src[..n]);
        n
    }
}
