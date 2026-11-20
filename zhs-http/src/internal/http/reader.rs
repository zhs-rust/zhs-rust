use std::io::{self, BufReader};

use crate::internal::error::HttpError;

use super::parse::{HttpLimits, RequestHead, read_request_head};
use super::reactor::AsyncTcpStream;

pub(crate) const READ_BUFFER: usize = 8 * 1024;

#[derive(Clone, Copy)]
pub(crate) enum LineKind {
    ChunkSize,

    Trailer,
}

impl LineKind {
    fn too_long(self) -> HttpError {
        match self {
            LineKind::ChunkSize => HttpError::InvalidChunk,
            LineKind::Trailer => HttpError::HeadTooLarge,
        }
    }

    fn malformed(self) -> HttpError {
        match self {
            LineKind::ChunkSize => HttpError::InvalidChunk,
            LineKind::Trailer => HttpError::InvalidHeader,
        }
    }
}

pub(crate) struct ConnectionReader {
    stream: AsyncTcpStream,
    buffer: Vec<u8>,
    start: usize,

    scratch: Vec<u8>,
}

impl ConnectionReader {
    pub(crate) fn new(stream: AsyncTcpStream) -> Self {
        Self {
            stream,
            buffer: Vec::new(),
            start: 0,
            scratch: vec![0; READ_BUFFER],
        }
    }

    pub(crate) fn available(&self) -> &[u8] {
        &self.buffer[self.start..]
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.available().is_empty()
    }

    pub(crate) fn consume(&mut self, count: usize) {
        self.start += count;

        if self.start >= self.buffer.len() {
            self.buffer.clear();
            self.start = 0;
        } else if self.start >= READ_BUFFER && self.start * 2 >= self.buffer.len() {
            self.buffer.drain(..self.start);
            self.start = 0;
        }
    }

    pub(crate) async fn fill_more(&mut self) -> Result<usize, HttpError> {
        match self.stream.read(&mut self.scratch).await {
            Ok(0) => Ok(0),
            Ok(read) => {
                self.buffer.extend_from_slice(&self.scratch[..read]);
                Ok(read)
            }
            Err(error) => Err(map_read_error(error)),
        }
    }

    pub(crate) async fn read_head(
        &mut self,
        limits: &HttpLimits,
    ) -> Result<Option<RequestHead>, HttpError> {
        loop {
            let offset = skip_leading_crlf(self.available());
            if let Some(end) = find_head_end(&self.available()[offset..]) {
                let mut reader = BufReader::new(&self.available()[offset..offset + end]);
                let head = read_request_head(&mut reader, limits)?.ok_or(HttpError::Incomplete)?;
                self.consume(offset + end);
                return Ok(Some(head));
            }

            if self.available().len() > limits.max_head_bytes {
                return Err(HttpError::HeadTooLarge);
            }

            if self.fill_more().await? == 0 {
                return if self.is_empty() {
                    Ok(None)
                } else {
                    Err(HttpError::Incomplete)
                };
            }
        }
    }

    pub(crate) async fn read_line(
        &mut self,
        max: usize,
        kind: LineKind,
    ) -> Result<Vec<u8>, HttpError> {
        loop {
            if let Some(position) = self.available().iter().position(|&byte| byte == b'\n') {
                let mut line = self.available()[..position].to_vec();
                self.consume(position + 1);

                if line.len() > max {
                    return Err(kind.too_long());
                }
                if line.last() != Some(&b'\r') {
                    return Err(kind.malformed());
                }
                line.pop();
                return Ok(line);
            }

            if self.available().len() > max {
                return Err(kind.too_long());
            }
            if self.fill_more().await? == 0 {
                return Err(HttpError::Incomplete);
            }
        }
    }

    pub(crate) async fn read_exact_buffer(&mut self, output: &mut [u8]) -> Result<(), HttpError> {
        let mut filled = 0;

        if !self.available().is_empty() {
            let take = self.available().len().min(output.len());
            output[..take].copy_from_slice(&self.available()[..take]);
            self.consume(take);
            filled = take;
        }

        if filled < output.len() {
            self.stream
                .read_exact(&mut output[filled..])
                .await
                .map_err(map_read_error)?;
        }

        Ok(())
    }

    pub(crate) fn shutdown(&mut self) {
        self.stream.shutdown();
    }
}

fn find_head_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

// read_request_head tolerates up to four empty lines before the request-line;
// skip that many so find_head_end does not stop at the empty lines themselves.
fn skip_leading_crlf(buffer: &[u8]) -> usize {
    let mut offset = 0;
    for _ in 0..4 {
        if buffer[offset..].starts_with(b"\r\n") {
            offset += 2;
        } else {
            break;
        }
    }
    offset
}

pub(crate) fn map_read_error(error: io::Error) -> HttpError {
    match error.kind() {
        io::ErrorKind::UnexpectedEof => HttpError::Incomplete,
        _ => HttpError::from_io(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{find_head_end, skip_leading_crlf};

    #[test]
    fn head_end_search_skips_leading_empty_lines() {
        let buffer = b"\r\n\r\nGET / HTTP/1.1\r\nHost: a\r\n\r\n";
        let offset = skip_leading_crlf(buffer);
        let end = find_head_end(&buffer[offset..]).expect("head end");
        assert_eq!(&buffer[offset..offset + end], b"GET / HTTP/1.1\r\nHost: a\r\n\r\n");

        // A single empty line is also tolerated without stopping the search.
        let buffer = b"\r\nGET / HTTP/1.1\r\nHost: a\r\n\r\n";
        let offset = skip_leading_crlf(buffer);
        assert_eq!(offset, 2);
        assert_eq!(
            find_head_end(&buffer[offset..]),
            Some(buffer.len() - offset)
        );
    }
}
