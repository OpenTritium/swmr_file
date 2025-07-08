use std::io::SeekFrom;
use swmr_file::{
    file_opt::FileOpt, get_reader::GetReader, get_writer::GetWriter,
    strategy::ImmediateSyncStrategy, sync_readable::SyncReadable, sync_writable::SyncWritable,
    write_read_file::WriteReadFile,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

#[tokio::main]
async fn main() {
    // 做好文件名消毒，windows 上遇到冒号就寄了
    let f = WriteReadFile::create("./Geshin Impact.txt".into()).unwrap();
    let mut w = f.get_writer(ImmediateSyncStrategy);
    let mut r = f.get_reader();
    w.write_all("我爱玩原神，也爱写 Rust。".as_bytes())
        .await
        .unwrap();
    println!("当前游标 {:?}", w.stream_position().await.unwrap());
    w.set_len(50).await.unwrap();
    println!(
        "setlen(50)  后游标的位置也不应当改变 {:?}",
        w.stream_position().await
    );
    w.seek(SeekFrom::Start(0)).await.unwrap();
    let mut buf = String::new();
    dbg!(w.read_to_string(&mut buf).await.unwrap());
    println!("{buf}");
    println!("当前游标 {:?}", w.stream_position().await.unwrap());
    let mut buf = String::new();
    r.read_to_string(&mut buf).await.unwrap();
    println!("读取器：{buf}, len:{}", buf.len());
    r.seek(SeekFrom::Start(0)).await.unwrap();
    let mut buf = vec![];
    w.seek(SeekFrom::Start(0)).await.unwrap();
    w.sync_write_all(b"hello world").await.unwrap();
    r.sync_read_to_end(&mut buf).await.unwrap();
    let s = String::from_utf8(buf).unwrap();
    println!("读取器 sync：{s}, len:{}", s.len());
}
