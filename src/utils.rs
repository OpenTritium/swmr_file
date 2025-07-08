use tokio::{
    io::{Error as IoError, ErrorKind as IoErrorKind, Result as IoResult},
    task::JoinHandle,
};

pub(crate) async fn asyncify<F, T>(f: F) -> IoResult<T>
where
    F: FnOnce() -> IoResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|err| Err(IoError::new(IoErrorKind::Other, err)))
}

#[inline(always)]
pub fn spawn_mandatory_blocking<F, R>(f: F) -> Option<JoinHandle<R>>
where
    R: Send + 'static,
    F: (FnOnce() -> R) + Send + 'static,
{
    let handle = tokio::runtime::Handle::try_current().ok()?;
    Some(handle.spawn_blocking(f))
}

#[inline(always)]
pub(crate) fn new_io_other_err(msg: &str) -> IoError {
    IoError::new(IoErrorKind::Other, msg)
}
