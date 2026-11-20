use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::internal::error::{Error, HttpError};
use zhs_mn::scheduler::Scheduler;

use super::parse::{BodyFraming, HttpLimits, MAX_CHUNK_LINE, parse_chunk_size, parse_header_line};
use super::reader::{ConnectionReader, LineKind};
use super::status::StatusCode;

pub type BodyChunk = Vec<u8>;

pub type BodyChunkFuture<'a> =
    Pin<Box<dyn Future<Output = Option<Result<BodyChunk, Error>>> + Send + 'a>>;

pub trait BodyStream: Send {
    fn next_chunk(&mut self) -> BodyChunkFuture<'_>;
}

#[derive(Clone, Copy, Debug)]
struct BodyLimits {
    max_body_bytes: u64,
    max_chunk_bytes: usize,
    max_header_line: usize,
    max_header_count: usize,
    max_head_bytes: usize,
}

impl BodyLimits {
    fn new(limits: &HttpLimits) -> Self {
        Self {
            max_body_bytes: limits.max_body_bytes as u64,
            max_chunk_bytes: limits.max_chunk_bytes.max(1),
            max_header_line: limits.max_header_line,
            max_header_count: limits.max_header_count,
            max_head_bytes: limits.max_head_bytes,
        }
    }
}

pub(crate) struct BodyShared {
    reader: Mutex<Option<ConnectionReader>>,
    finished: AtomicBool,
    failure: AtomicU16,
}

impl BodyShared {
    pub(crate) fn new() -> Self {
        Self {
            reader: Mutex::new(None),
            finished: AtomicBool::new(false),
            failure: AtomicU16::new(0),
        }
    }

    pub(crate) fn take_reader(&self) -> Option<ConnectionReader> {
        lock(&self.reader).take()
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub(crate) fn failure_status(&self) -> Option<StatusCode> {
        StatusCode::new(self.failure.load(Ordering::Acquire))
    }

    fn mark_finished(&self) {
        self.finished.store(true, Ordering::Release);
    }

    fn mark_failed(&self, status: StatusCode) {
        self.failure.store(status.as_u16(), Ordering::Release);
    }

    fn return_reader(&self, reader: ConnectionReader) {
        *lock(&self.reader) = Some(reader);
    }
}

pub struct RequestBody {
    source: BodySource,
    limits: BodyLimits,
    total: u64,
    finished: bool,
    shared: Option<Arc<BodyShared>>,
    scheduler: Option<Scheduler>,
    read_timeout: Duration,
}

enum BodySource {
    Empty,
    Memory {
        data: Vec<u8>,
        position: usize,
    },
    Stream {
        reader: Option<ConnectionReader>,
        framing: Framing,
    },
}

enum Framing {
    ContentLength { remaining: u64 },
    Chunked { phase: ChunkPhase },
}

#[derive(Clone, Copy)]
enum ChunkPhase {
    NeedSize,
    Data { remaining: u64 },
    DataCrlf,
    Trailers,
    Done,
}

impl RequestBody {
    pub fn empty() -> Self {
        Self {
            source: BodySource::Empty,
            limits: BodyLimits::new(&HttpLimits::default()),
            total: 0,
            finished: true,
            shared: None,
            scheduler: None,
            read_timeout: Duration::from_secs(30),
        }
    }

    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        let data = bytes.into();
        let finished = data.is_empty();
        Self {
            source: BodySource::Memory { data, position: 0 },
            limits: BodyLimits::new(&HttpLimits::default()),
            total: 0,
            finished,
            shared: None,
            scheduler: None,
            read_timeout: Duration::from_secs(30),
        }
    }

    pub(crate) fn streaming(
        reader: ConnectionReader,
        framing: BodyFraming,
        limits: &HttpLimits,
        scheduler: Scheduler,
        read_timeout: Duration,
        shared: Arc<BodyShared>,
    ) -> Self {
        let limits = BodyLimits::new(limits);

        let (source, finished) = match framing {
            BodyFraming::None | BodyFraming::ContentLength(0) => {
                shared.return_reader(reader);
                (BodySource::Empty, true)
            }
            BodyFraming::ContentLength(length) => (
                BodySource::Stream {
                    reader: Some(reader),
                    framing: Framing::ContentLength { remaining: length },
                },
                false,
            ),
            BodyFraming::Chunked => (
                BodySource::Stream {
                    reader: Some(reader),
                    framing: Framing::Chunked {
                        phase: ChunkPhase::NeedSize,
                    },
                },
                false,
            ),
        };

        if finished {
            shared.mark_finished();
        }

        Self {
            source,
            limits,
            total: 0,
            finished,
            shared: Some(shared),
            scheduler: Some(scheduler),
            read_timeout,
        }
    }

    pub async fn read_chunk(&mut self) -> crate::internal::error::Result<Option<BodyChunk>> {
        if self.finished {
            return Ok(None);
        }

        let limits = self.limits;
        let scheduler = self.scheduler.clone();
        let read_timeout = self.read_timeout;
        let source = &mut self.source;
        let total = &mut self.total;

        let outcome = match scheduler {
            Some(scheduler) => {
                match scheduler
                    .timeout(read_timeout, read_source(source, total, limits))
                    .await
                {
                    Ok(outcome) => outcome,
                    Err(_) => Err(Error::Http(HttpError::Timeout)),
                }
            }
            None => read_source(source, total, limits).await,
        };

        match outcome {
            Ok(Some(chunk)) => Ok(Some(chunk)),
            Ok(None) => {
                self.finished = true;
                if let Some(shared) = &self.shared {
                    shared.mark_finished();
                }
                Ok(None)
            }
            Err(error) => {
                if let Some(shared) = &self.shared {
                    let status = match &error {
                        Error::Http(http) => http.status(),
                        _ => StatusCode::INTERNAL_SERVER_ERROR,
                    };
                    shared.mark_failed(status);
                }
                Err(error)
            }
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub async fn bytes(&mut self) -> crate::internal::error::Result<Vec<u8>> {
        let mut collected = Vec::new();
        while let Some(chunk) = self.read_chunk().await? {
            collected.extend_from_slice(&chunk);
        }
        Ok(collected)
    }

    pub fn buffered(&self) -> Option<&[u8]> {
        match &self.source {
            BodySource::Memory { data, position } if !self.finished => Some(&data[*position..]),
            _ => None,
        }
    }
}

impl fmt::Debug for RequestBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match &self.source {
            BodySource::Empty => "empty",
            BodySource::Memory { data, .. } => {
                return formatter
                    .debug_struct("RequestBody")
                    .field("kind", &"memory")
                    .field("len", &data.len())
                    .field("finished", &self.finished)
                    .finish();
            }
            BodySource::Stream { .. } => "stream",
        };

        formatter
            .debug_struct("RequestBody")
            .field("kind", &kind)
            .field("finished", &self.finished)
            .field("bytes_read", &self.total)
            .finish()
    }
}

impl Drop for RequestBody {
    fn drop(&mut self) {
        let BodySource::Stream { reader, .. } = &mut self.source else {
            return;
        };
        let Some(shared) = &self.shared else {
            return;
        };

        if let Some(reader) = reader.take() {
            shared.return_reader(reader);
        }
    }
}

async fn read_source(
    source: &mut BodySource,
    total: &mut u64,
    limits: BodyLimits,
) -> Result<Option<BodyChunk>, Error> {
    match source {
        BodySource::Empty => Ok(None),
        BodySource::Memory { data, position } => {
            if *position >= data.len() {
                return Ok(None);
            }
            let end = (*position + limits.max_chunk_bytes).min(data.len());
            let chunk = data[*position..end].to_vec();
            *position = end;
            Ok(Some(chunk))
        }
        BodySource::Stream { reader, framing } => {
            let reader = reader.as_mut().ok_or(Error::Http(HttpError::Incomplete))?;
            read_stream(reader, framing, total, limits)
                .await
                .map_err(Error::Http)
        }
    }
}

async fn read_stream(
    reader: &mut ConnectionReader,
    framing: &mut Framing,
    total: &mut u64,
    limits: BodyLimits,
) -> Result<Option<BodyChunk>, HttpError> {
    match framing {
        Framing::ContentLength { remaining } => {
            if *remaining == 0 {
                return Ok(None);
            }

            let take = (*remaining).min(limits.max_chunk_bytes as u64);
            check_budget(*total, take, limits.max_body_bytes)?;

            let mut chunk = vec![0; take as usize];
            reader.read_exact_buffer(&mut chunk).await?;
            *remaining -= take;
            *total += take;
            Ok(Some(chunk))
        }
        Framing::Chunked { phase } => loop {
            match *phase {
                ChunkPhase::NeedSize => {
                    let line = reader
                        .read_line(MAX_CHUNK_LINE, LineKind::ChunkSize)
                        .await?;
                    let size = parse_chunk_size(&line)?;

                    if size == 0 {
                        *phase = ChunkPhase::Trailers;
                        continue;
                    }

                    check_budget(*total, size, limits.max_body_bytes)?;
                    *phase = ChunkPhase::Data { remaining: size };
                }
                ChunkPhase::Data { remaining } => {
                    if remaining == 0 {
                        *phase = ChunkPhase::DataCrlf;
                        continue;
                    }

                    let take = remaining.min(limits.max_chunk_bytes as u64);
                    let mut chunk = vec![0; take as usize];
                    reader.read_exact_buffer(&mut chunk).await?;
                    *total += take;
                    *phase = ChunkPhase::Data {
                        remaining: remaining - take,
                    };
                    return Ok(Some(chunk));
                }
                ChunkPhase::DataCrlf => {
                    let mut terminator = [0u8; 2];
                    reader.read_exact_buffer(&mut terminator).await?;
                    if terminator != *b"\r\n" {
                        return Err(HttpError::InvalidChunk);
                    }
                    *phase = ChunkPhase::NeedSize;
                }
                ChunkPhase::Trailers => {
                    read_trailers(reader, limits).await?;
                    *phase = ChunkPhase::Done;
                    return Ok(None);
                }
                ChunkPhase::Done => return Ok(None),
            }
        },
    }
}

fn check_budget(total: u64, additional: u64, max: u64) -> Result<(), HttpError> {
    if total > max || additional > max - total {
        return Err(HttpError::BodyTooLarge);
    }
    Ok(())
}

async fn read_trailers(reader: &mut ConnectionReader, limits: BodyLimits) -> Result<(), HttpError> {
    let mut trailer_bytes: usize = 0;
    let mut trailer_count = 0;

    loop {
        let line = reader
            .read_line(limits.max_header_line, LineKind::Trailer)
            .await?;
        trailer_bytes = trailer_bytes.saturating_add(line.len() + 2);
        if trailer_bytes > limits.max_head_bytes {
            return Err(HttpError::HeadTooLarge);
        }
        if line.is_empty() {
            return Ok(());
        }

        trailer_count += 1;
        if trailer_count > limits.max_header_count {
            return Err(HttpError::TooManyHeaders);
        }
        parse_header_line(&line)?;
    }
}

pub enum ResponseBody {
    Bytes(Vec<u8>),

    Stream(Box<dyn BodyStream>),
}

impl ResponseBody {
    pub fn empty() -> Self {
        Self::Bytes(Vec::new())
    }

    pub fn bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(bytes.into())
    }

    pub fn stream(stream: impl BodyStream + 'static) -> Self {
        Self::Stream(Box::new(stream))
    }

    pub fn chunks(chunks: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self::stream(ChunkList::new(chunks))
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            ResponseBody::Bytes(bytes) => Some(bytes),
            ResponseBody::Stream(_) => None,
        }
    }

    pub fn len(&self) -> Option<usize> {
        self.as_bytes().map(<[u8]>::len)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == Some(0)
    }

    pub fn is_stream(&self) -> bool {
        matches!(self, ResponseBody::Stream(_))
    }
}

impl fmt::Debug for ResponseBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResponseBody::Bytes(bytes) => formatter
                .debug_struct("Bytes")
                .field("len", &bytes.len())
                .finish(),
            ResponseBody::Stream(_) => formatter.write_str("Stream(..)"),
        }
    }
}

impl From<Vec<u8>> for ResponseBody {
    fn from(bytes: Vec<u8>) -> Self {
        ResponseBody::Bytes(bytes)
    }
}

impl From<&str> for ResponseBody {
    fn from(text: &str) -> Self {
        ResponseBody::Bytes(text.as_bytes().to_vec())
    }
}

pub struct ChunkList {
    chunks: std::collections::VecDeque<BodyChunk>,
}

impl ChunkList {
    pub fn new(chunks: impl IntoIterator<Item = BodyChunk>) -> Self {
        Self {
            chunks: chunks.into_iter().collect(),
        }
    }
}

impl BodyStream for ChunkList {
    fn next_chunk(&mut self) -> BodyChunkFuture<'_> {
        Box::pin(async move { self.chunks.pop_front().map(Ok) })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(crate::internal::recover_poison)
}
