use std::io::{ErrorKind as IoErrorKind, Read, Result as IoResult};
use sync_file::{Size, SyncFile};
use tokio::io::ReadBuf;

pub(crate) type RingBuffer = slice_ring_buffer::SliceRingBuffer<u8>;

pub(crate) trait RingBufferExt {
    fn obsolete(&mut self) -> isize;
    fn read_file(&mut self, src: &mut SyncFile) -> IoResult<usize>;
    fn write_into(&mut self, dst: &mut ReadBuf<'_>) -> usize;
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

pub(crate) const BUFFER_MAX_SIZE: usize = 64 * 0x400; // 32KB

impl RingBufferExt for RingBuffer {
    #[inline(always)]
    /// 丢弃该缓冲中的所有数据，并返回一个负数以表示已丢弃的字节数
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
    /// `src` 有游标，所以你下次读取会从上次结束的位置开始。
    fn read_file(&mut self, src: &mut SyncFile) -> IoResult<usize> {
        debug_assert!(self.is_empty());
        let remain = (src.size()? - src.offset()) as usize;
        let n = BUFFER_MAX_SIZE.min(remain);
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
                self.clear();
            })
    }

    #[inline(always)]
    /// 此函数不影响缓冲区大小（self 与 dst），
    /// 每次写入量在 `[self.len(), dst.remaining()]` 间，
    /// 你可以持续调用此函数，直到 self 被消耗殆尽
    fn write_into(&mut self, dst: &mut ReadBuf<'_>) -> usize {
        let n = self.len().min(dst.remaining()); // 保证 put_slice 安全
        dst.put_slice(&self[..n]);
        unsafe {
            self.move_head_unchecked(
                n.try_into().expect(
                    "将 ringbuf 的缓冲拷贝到 dst 中时，协商字节数量 usize 转换到 isize 时溢出",
                ),
            )
        };
        n
    }

    #[inline(always)]
    /// 读取一部分 src，读取的字节数量在 `[src.len(), BUFFER_MAX_SIZE]` 之间，
    /// 调用此函数之间请保证本缓冲区已被清理完毕。
    fn read_from(&mut self, src: &[u8]) -> usize {
        debug_assert!(self.is_empty());
        let n = BUFFER_MAX_SIZE.min(src.len());
        self.extend_from_slice(&src[..n]);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slice_ring_buffer::sdeq;
    use std::io::{Seek, SeekFrom, Write};
    use tempfile::tempfile;

    fn mock_8m_data() -> Vec<u8> {
        (0..8 * 0x400 * 0x400)
            .map(|_| rand::random::<u8>())
            .collect::<Vec<_>>()
    }

    #[test]
    fn obsolete() {
        const LEN: usize = 1024;
        let mut buf = sdeq![0; LEN];
        assert_eq!(buf.obsolete(), -(LEN as isize));
    }

    #[test]
    fn read_small_file() -> IoResult<()> {
        let file = tempfile()?;
        let mut file = SyncFile::from(file);
        const CONTENT: &[u8] = b"some bytes here";
        file.write_all(CONTENT)?;
        let file_len = file.offset() as usize;
        file.seek(SeekFrom::Start(0))?;
        let mut buf = RingBuffer::new();
        let buf_len = buf.read_file(&mut file)?;
        assert_eq!(file_len, buf_len);
        assert_eq!(CONTENT, buf.as_slice());
        Ok(())
    }

    #[test]
    fn read_large_file() -> IoResult<()> {
        let content = mock_8m_data();
        let file = tempfile()?;
        let mut file = SyncFile::from(file);
        file.write_all(&content)?;
        file.seek(SeekFrom::Start(0))?;
        let mut buf = RingBuffer::new();
        let mut start = 0;
        let mut round = 0;
        loop {
            let buf_len = buf.read_file(&mut file)?;
            assert_eq!(
                &content[start..{
                    start += buf_len;
                    start
                }],
                buf.as_slice()
            );
            // consume buf
            buf.clear();
            round += 1;
            if file.offset() == file.size()? {
                break;
            }
        }
        assert_eq!(round, content.len() / BUFFER_MAX_SIZE);
        Ok(())
    }

    #[test]
    fn ship_large_data() {
        let src = mock_8m_data();
        let mut dst: Vec<u8> = Vec::with_capacity(src.len());
        unsafe { dst.set_len(src.len()) };
        let mut dst = ReadBuf::new(&mut dst);
        let mut buf = RingBuffer::with_capacity(BUFFER_MAX_SIZE);
        let mut offset = 0;
        while offset < src.len() {
            let n = buf.read_from(&src[offset..]);
            offset += n;
            let written = buf.write_into(&mut dst);
            assert_eq!(n, written);
        }
        assert_eq!(offset, 8 * 0x400 * 0x400);
        let dst = dst.filled();
        assert_eq!(src, dst);
    }
}
