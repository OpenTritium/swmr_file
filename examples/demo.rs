use std::{io::SeekFrom, thread::park};

use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use worm_file::{
    file::{Writable, WriteReadFile},
    strategy::ImmediateSyncStrategy,
};

#[tokio::main]
async fn main() {
    let f = WriteReadFile::new("./a.txt".into());
    let mut w = f.get_writer(ImmediateSyncStrategy);
    w.write_all(b"adafeaffsagfageargaergrghsgsre")
        .await
        .unwrap();
    w.set_len(50).await.unwrap();
    let mut v = vec![];
    let result = w.seek(SeekFrom::Start(2)).await.unwrap();
    println!(
        "seeked:{},{}",
        result,
        w.seek(SeekFrom::Current(0)).await.unwrap()
    );
    w.read_to_end(&mut v).await.unwrap();
    park();
}
