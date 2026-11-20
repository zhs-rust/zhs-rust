use std::future::Future;
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use crate::internal::error::{Error, HttpError};
use zhs_mn::scheduler::Scheduler;

use super::body::{BodyShared, RequestBody, ResponseBody};
use super::reactor::{AsyncTcpStream, Reactor};
use super::reader::ConnectionReader;
use super::response::Response;
use super::server::{Handler, ServerConfig, ServerState};
use super::status::StatusCode;
use super::version::Version;
use super::websocket::{WebSocket, WebSocketHandler, WebSocketLimits};
use super::write::{EncodedHead, encode_response, error_response};

#[allow(clippy::collapsible_if)]
pub(crate) async fn serve<H>(
    stream: TcpStream,
    peer: SocketAddr,
    handler: &H,
    config: &ServerConfig,
    scheduler: &Scheduler,
    reactor: &Reactor,
    state: &ServerState,
) where
    H: Handler,
{
    let local_addr = stream.local_addr().ok();
    let stream = match AsyncTcpStream::new(stream, reactor) {
        Ok(stream) => stream,
        Err(_) => {
            state.record_failure();
            return;
        }
    };
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(_) => {
            state.record_failure();
            return;
        }
    };
    let mut reader = Some(ConnectionReader::new(stream));
    let mut served = 0;

    loop {
        if served > 0 && state.is_shutdown() {
            break;
        }

        let Some(mut active) = reader.take() else {
            break;
        };

        let head_timeout = if served == 0 {
            config.read_timeout
        } else {
            config.keep_alive_timeout
        };

        let head = match scheduler
            .timeout(head_timeout, active.read_head(&config.limits))
            .await
        {
            Ok(Ok(Some(head))) => head,
            Ok(Ok(None)) => break,
            Ok(Err(error)) => {
                if state.is_shutdown() {
                    break;
                }
                state.record_failure();
                write_error(&mut writer, &error, scheduler, config.write_timeout).await;
                break;
            }
            Err(_) => {
                if state.is_shutdown() || (served > 0 && active.is_empty()) {
                    break;
                }
                state.record_failure();
                write_error(
                    &mut writer,
                    &HttpError::Timeout,
                    scheduler,
                    config.write_timeout,
                )
                .await;
                break;
            }
        };

        if let Some(length) = head.framing().content_length()
            && length > config.limits.max_body_bytes as u64
        {
            state.record_failure();
            write_error(
                &mut writer,
                &HttpError::BodyTooLarge,
                scheduler,
                config.write_timeout,
            )
            .await;
            break;
        }

        if let Some(expectation) = head.headers().get("expect") {
            if head.version().is_http11() {
                if expectation.trim().eq_ignore_ascii_case("100-continue") {
                    if head.has_body()
                        && !write_bytes(
                            &mut writer,
                            b"HTTP/1.1 100 Continue\r\n\r\n",
                            scheduler,
                            config.write_timeout,
                        )
                        .await
                    {
                        state.record_failure();
                        break;
                    }
                } else {
                    state.record_failure();
                    write_error(
                        &mut writer,
                        &HttpError::UnsupportedExpectation,
                        scheduler,
                        config.write_timeout,
                    )
                    .await;
                    break;
                }
            }
        }

        let shared = Arc::new(BodyShared::new());
        let body = RequestBody::streaming(
            active,
            head.framing(),
            &config.limits,
            scheduler.clone(),
            config.read_timeout,
            Arc::clone(&shared),
        );

        let mut request = head.into_request(body);
        request.set_connection_info(peer, local_addr);

        let version = request.version();
        let head_only = request.method().is_head();

        let mut keep_alive = if version.is_http11() {
            !request.connection_close()
        } else {
            request.connection_keep_alive() && !request.connection_close()
        };

        let outcome = match catch_unwind(AssertUnwindSafe(|| handler.handle(request))) {
            Ok(future) => CatchPanic::new(future).await,
            Err(_) => Err(()),
        };

        let mut recovered = shared.take_reader();
        let body_finished = shared.is_finished();
        let body_failure = shared.failure_status();

        let mut response = match outcome {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                state.record_failure();
                response_for_error(&error)
            }
            Err(_) => {
                state.record_failure();
                error_response(StatusCode::INTERNAL_SERVER_ERROR)
            }
        };

        if let Some(status) = body_failure {
            state.record_failure();
            response = error_response(status);
            keep_alive = false;
        }

        let upgrade = response.take_websocket();
        if upgrade.is_some() {
            keep_alive = false;
        }

        keep_alive &= !response.close_connection() && !state.is_shutdown() && body_finished;
        if config.max_requests_per_connection != 0
            && served + 1 >= config.max_requests_per_connection
        {
            keep_alive = false;
        }

        let send_body = !head_only && !response.status().forbids_body();
        let streaming = response.body().is_stream();
        if streaming && send_body && version == Version::Http10 {
            keep_alive = false;
        }

        reader = if keep_alive { recovered.take() } else { None };
        if keep_alive && reader.is_none() {
            keep_alive = false;
        }

        let encoded = match encode_response(&response, version, keep_alive, head_only) {
            Ok(encoded) => encoded,
            Err(_) => {
                state.record_failure();
                break;
            }
        };

        let written = write_response(
            &mut writer,
            &mut response,
            &encoded,
            send_body,
            scheduler,
            config.write_timeout,
        )
        .await;

        if written.is_err() {
            state.record_failure();
            break;
        }

        if let Some(upgrade) = upgrade {
            state.request_completed();

            let Some(reader) = recovered.take() else {
                break;
            };
            if !body_finished {
                let mut reader = reader;
                reader.shutdown();
                break;
            }

            let failed = run_websocket(
                upgrade,
                reader,
                writer,
                scheduler.clone(),
                config.write_timeout,
            )
            .await;
            if failed {
                state.record_failure();
            }

            return;
        }

        state.request_completed();
        served += 1;
        if !keep_alive {
            break;
        }
    }

    if let Some(mut reader) = reader {
        reader.shutdown();
    }
    writer.shutdown();
}

async fn run_websocket(
    upgrade: Box<dyn WebSocketHandler>,
    reader: ConnectionReader,
    writer: AsyncTcpStream,
    scheduler: Scheduler,
    write_timeout: Duration,
) -> bool {
    let socket = WebSocket::new(
        reader,
        writer,
        scheduler,
        WebSocketLimits::default(),
        write_timeout,
    );

    Guard::new(upgrade.run(socket)).await.is_err()
}

async fn write_response(
    writer: &mut AsyncTcpStream,
    response: &mut Response,
    encoded: &EncodedHead,
    send_body: bool,
    scheduler: &Scheduler,
    timeout: Duration,
) -> io::Result<()> {
    write_all_timed(scheduler, timeout, writer, &encoded.bytes).await?;

    if !send_body {
        return Ok(());
    }

    match response.body_mut() {
        ResponseBody::Bytes(bytes) => {
            if bytes.is_empty() {
                return Ok(());
            }
            write_all_timed(scheduler, timeout, writer, bytes).await
        }
        ResponseBody::Stream(stream) => {
            if encoded.chunked {
                loop {
                    match next_chunk(scheduler, timeout, stream.as_mut()).await? {
                        Some(chunk) => {
                            if chunk.is_empty() {
                                continue;
                            }
                            let size = format!("{:X}\r\n", chunk.len());
                            let written = scheduler
                                .timeout(timeout, async {
                                    writer.write_all(size.as_bytes()).await?;
                                    writer.write_all(&chunk).await?;
                                    writer.write_all(b"\r\n").await
                                })
                                .await;
                            match written {
                                Ok(result) => result?,
                                Err(_) => return Err(write_timeout()),
                            }
                        }
                        None => {
                            write_all_timed(scheduler, timeout, writer, b"0\r\n\r\n").await?;
                            return Ok(());
                        }
                    }
                }
            } else if let Some(remaining) = encoded.content_length {
                write_stream_with_length(writer, stream.as_mut(), remaining, scheduler, timeout)
                    .await
            } else {
                while let Some(chunk) = next_chunk(scheduler, timeout, stream.as_mut()).await? {
                    write_all_timed(scheduler, timeout, writer, &chunk).await?;
                }
                Ok(())
            }
        }
    }
}

async fn write_stream_with_length(
    writer: &mut AsyncTcpStream,
    stream: &mut dyn super::body::BodyStream,
    mut remaining: u64,
    scheduler: &Scheduler,
    timeout: Duration,
) -> io::Result<()> {
    while remaining > 0 {
        match next_chunk(scheduler, timeout, stream).await? {
            Some(chunk) => {
                if chunk.is_empty() {
                    continue;
                }
                let length = chunk.len() as u64;
                if length > remaining {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "response body exceeds the declared Content-Length",
                    ));
                }
                write_all_timed(scheduler, timeout, writer, &chunk).await?;
                remaining -= length;
            }
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "response body ended before the declared Content-Length",
                ));
            }
        }
    }

    loop {
        match next_chunk(scheduler, timeout, stream).await? {
            Some(chunk) if chunk.is_empty() => continue,
            Some(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "response body exceeds the declared Content-Length",
                ));
            }
            None => return Ok(()),
        }
    }
}

async fn next_chunk(
    scheduler: &Scheduler,
    timeout: Duration,
    stream: &mut dyn super::body::BodyStream,
) -> io::Result<Option<Vec<u8>>> {
    match scheduler.timeout(timeout, stream.next_chunk()).await {
        Ok(Some(Ok(chunk))) => Ok(Some(chunk)),
        Ok(Some(Err(error))) => Err(io::Error::other(error.to_string())),
        Ok(None) => Ok(None),
        Err(_) => Err(write_timeout()),
    }
}

fn write_timeout() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "timed out while writing the response",
    )
}

async fn write_all_timed(
    scheduler: &Scheduler,
    timeout: Duration,
    writer: &mut AsyncTcpStream,
    bytes: &[u8],
) -> io::Result<()> {
    match scheduler.timeout(timeout, writer.write_all(bytes)).await {
        Ok(result) => result,
        Err(_) => Err(write_timeout()),
    }
}

async fn write_bytes(
    writer: &mut AsyncTcpStream,
    bytes: &[u8],
    scheduler: &Scheduler,
    timeout: Duration,
) -> bool {
    write_all_timed(scheduler, timeout, writer, bytes)
        .await
        .is_ok()
}

async fn write_error(
    writer: &mut AsyncTcpStream,
    error: &HttpError,
    scheduler: &Scheduler,
    timeout: Duration,
) {
    let response = error_response(error.status());
    let Ok(encoded) = encode_response(&response, Version::Http11, false, false) else {
        return;
    };

    let _ = write_all_timed(scheduler, timeout, writer, &encoded.bytes).await;
    if let Some(bytes) = response.body().as_bytes() {
        let _ = write_all_timed(scheduler, timeout, writer, bytes).await;
    }
    writer.shutdown();
}

fn response_for_error(error: &Error) -> Response {
    match error {
        Error::Http(http) => error_response(http.status()),
        _ => error_response(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

struct Guard<F> {
    future: Pin<Box<F>>,
}

impl<F> Guard<F> {
    fn new(future: F) -> Self {
        Self {
            future: Box::pin(future),
        }
    }
}

impl<F> Future for Guard<F>
where
    F: Future<Output = ()>,
{
    type Output = std::result::Result<(), ()>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match catch_unwind(AssertUnwindSafe(|| this.future.as_mut().poll(context))) {
            Ok(Poll::Ready(())) => Poll::Ready(Ok(())),
            Ok(Poll::Pending) => Poll::Pending,
            Err(_) => Poll::Ready(Err(())),
        }
    }
}

struct CatchPanic<F> {
    future: Pin<Box<F>>,
}

impl<F> CatchPanic<F> {
    fn new(future: F) -> Self {
        Self {
            future: Box::pin(future),
        }
    }
}

impl<F> Future for CatchPanic<F>
where
    F: Future<Output = Result<Response, Error>>,
{
    type Output = Result<Result<Response, Error>, ()>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match catch_unwind(AssertUnwindSafe(|| this.future.as_mut().poll(context))) {
            Ok(Poll::Ready(result)) => Poll::Ready(Ok(result)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(_) => Poll::Ready(Err(())),
        }
    }
}
