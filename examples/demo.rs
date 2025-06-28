use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use worm_file::{
    file::{GetWriter, WriteReadFile},
    strategy::ImmediateSyncStrategy,
    utils::SyncWritable,
};

#[tokio::main]
async fn main() {
    // 做好文件名消毒，windows 上遇到冒号就寄了
    let f = WriteReadFile::create("./Geshin Impact.txt".into()).unwrap();
    let mut w = f.get_writer(ImmediateSyncStrategy);
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
}
