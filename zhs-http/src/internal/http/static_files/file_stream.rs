use std::fs::File;
use std::io::{self, Read};
use std::sync::{Arc, Mutex};

use crate::internal::error::{Error, HttpError};
use zhs_mn::thread_pool::ThreadPool;

use super::super::body::{BodyChunkFuture, BodyStream};

pub(super) struct FileStream {
    file: Arc<Mutex<Option<File>>>,
    pool: ThreadPool,
    remaining: u64,
    chunk_size: usize,
}

impl FileStream {
    pub(super) fn new(file: File, pool: ThreadPool, length: u64, chunk_size: usize) -> Self {
        Self {
            file: Arc::new(Mutex::new(Some(file))),
            pool,
            remaining: length,
            chunk_size: chunk_size.max(1),
        }
    }
}

impl BodyStream for FileStream {
    fn next_chunk(&mut self) -> BodyChunkFuture<'_> {
        if self.remaining == 0 {
            return Box::pin(async { None });
        }

        let available = usize::try_from(self.remaining).unwrap_or(usize::MAX);
        let wanted = self.chunk_size.min(available);
        self.remaining -= wanted as u64;

        let file = Arc::clone(&self.file);
        let pool = self.pool.clone();

        Box::pin(async move {
            let handle = match pool.try_spawn_blocking(move || read_chunk(&file, wanted)) {
                Ok(handle) => handle,
                Err(error) => {
                    return Some(Err(Error::Http(HttpError::from_io(io::Error::other(
                        error.to_string(),
                    )))));
                }
            };

            match handle.await {
                Ok(Ok(chunk)) if chunk.is_empty() => {
                    Some(Err(Error::Http(HttpError::from_io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the static file ended before its declared length",
                    )))))
                }
                Ok(Ok(chunk)) => Some(Ok(chunk)),
                Ok(Err(error)) => Some(Err(Error::Http(HttpError::from_io(error)))),
                Err(error) => Some(Err(Error::Http(HttpError::from_io(io::Error::other(
                    error.to_string(),
                ))))),
            }
        })
    }
}

fn read_chunk(file: &Mutex<Option<File>>, wanted: usize) -> io::Result<Vec<u8>> {
    let mut guard = file.lock().unwrap_or_else(crate::internal::recover_poison);
    let file = guard
        .as_mut()
        .ok_or_else(|| io::Error::other("the static file is already closed"))?;

    let mut buffer = vec![0; wanted];
    let mut filled = 0;
    while filled < wanted {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    buffer.truncate(filled);

    Ok(buffer)
}
