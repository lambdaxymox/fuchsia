// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::fragment_io::{new_fragment_io, FragmentReader, FragmentWriter};
use crate::lossy_text::LossyText;
use anyhow::{format_err, Context as _, Error};
use fuchsia_async::TimeoutExt;
use fuchsia_async::Timer;
use future::Either;
use futures::channel::mpsc;
use futures::prelude::*;
use overnet_core::{LinkReceiver, LinkSender, Router};
use std::sync::{Arc, Weak};
use std::time::Duration;
use stream_framer::{Deframer, Framer, ReadBytes};

#[derive(Clone, Copy, Debug)]
pub enum Role {
    Client,
    Server,
}

pub async fn run(
    role: Role,
    read: impl AsyncRead + Unpin + Send,
    write: impl AsyncWrite + Unpin + Send,
    router: Weak<Router>,
    skipped: impl AsyncWrite + Unpin + Send,
    descriptor: Option<&crate::descriptor::Descriptor>,
) -> Error {
    let router = WeakRouter(router);
    let mut file_handler = FileHandler { read, write, skipped };
    loop {
        let _ = file_handler
            .run(|fragment_reader, fragment_writer| async {
                if let Err(e) = main(
                    role,
                    fragment_reader,
                    fragment_writer,
                    &router,
                    descriptor.map(|d| format!("{}", d)),
                )
                .await
                {
                    tracing::warn!("serial handler failed: {:?}", e);
                }
                Ok(())
            })
            .await;
        if let Err(e) = file_handler
            .run(|fragment_reader, fragment_writer| reset(role, fragment_reader, fragment_writer))
            .await
        {
            return e;
        }
        fuchsia_async::Timer::new(Duration::from_millis(100)).await;
    }
}

struct FileHandler<R, W, S> {
    read: R,
    write: W,
    skipped: S,
}

impl<R: AsyncRead + Unpin + Send, W: AsyncWrite + Unpin + Send, S: AsyncWrite + Unpin + Send>
    FileHandler<R, W, S>
{
    // Build up a fragmenter -> framer -> bytes pipeline, and use that to run some inner function.
    // This allows us to drop in-flight framing work whenever the inner function finishes, whilst retaining
    // the backing files for future iterations.
    async fn run<'a, F, Fut>(&'a mut self, f: F) -> Result<(), Error>
    where
        F: FnOnce(StreamSplitter<&'a mut S>, FragmentWriter) -> Fut,
        Fut: Send + Future<Output = Result<(), Error>>,
    {
        let framer = Framer::new(LossyText);
        let (framer_writer, framer_reader) = mpsc::channel(0);

        let deframer = Deframer::new(LossyText);
        let (deframer_writer, deframer_reader) = mpsc::channel(0);

        let (fragment_writer, fragment_reader, fragment_io_runner) =
            new_fragment_io(framer, framer_writer, deframer_reader);
        let fragment_reader = StreamSplitter { fragment_reader, skipped_bytes: &mut self.skipped };

        let support = future::try_join3(
            write_bytes(framer_reader, &mut self.write),
            read_bytes(&mut self.read, deframer, deframer_writer),
            fragment_io_runner,
        )
        .map_ok(drop)
        .boxed();

        let fut = f(fragment_reader, fragment_writer).boxed();

        match future::select(fut, support).await {
            Either::Left((r, _)) => {
                eprintln!("main task finished: {:?}", r);
                r
            }
            Either::Right((r, m)) => {
                if let Some(r) = m.now_or_never() {
                    eprintln!("main task finished at the last moment: {:?}", r);
                    r
                } else {
                    eprintln!("support task finished: {:?}", r);
                    match r {
                        Err(e) => Err(e),
                        Ok(_) => unreachable!(),
                    }
                }
            }
        }
    }
}

struct WeakRouter(Weak<Router>);

impl WeakRouter {
    fn get(&self) -> Result<Arc<Router>, Error> {
        Weak::upgrade(&self.0).ok_or_else(|| format_err!("Router gone"))
    }
}

struct StreamSplitter<OutputSink> {
    fragment_reader: FragmentReader,
    skipped_bytes: OutputSink,
}

impl<OutputSink: AsyncWrite + Unpin> StreamSplitter<OutputSink> {
    async fn read(&mut self) -> Result<Vec<u8>, Error> {
        loop {
            match self.fragment_reader.read().await? {
                ReadBytes::Unframed(frame) => self.skipped_bytes.write_all(&frame).await?,
                ReadBytes::Framed(frame) => return Ok(frame),
            }
        }
    }
}

async fn write_bytes(
    mut framer_reader: mpsc::Receiver<Vec<u8>>,
    mut f_write: impl AsyncWrite + Unpin,
) -> Result<(), Error> {
    while let Some(bytes) = framer_reader.next().await {
        tracing::trace!("SERIAL WRITE: {:?}", bytes);
        f_write.write_all(&bytes).await.context("serial.write_all failed")?;
        tracing::trace!("WRITE COMPLETE");
        f_write.flush().await?;
        tracing::trace!("FLUSHED");
    }

    Ok(())
}

async fn read_bytes(
    mut f_read: impl AsyncRead + Unpin,
    mut deframer: Deframer<LossyText>,
    mut deframer_writer: mpsc::Sender<ReadBytes>,
) -> Result<(), Error> {
    const INCOMING_BYTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

    let mut buf = [0u8; 1024];
    let mut parse_frame_timeout = None;

    loop {
        tracing::trace!("SERIAL READ");

        let read_fut = f_read.read(&mut buf);

        let bytes_read = if let Some(timeout) = parse_frame_timeout.take() {
            // If we have a timeout, then that means we have some pending bytes in the deframer. If
            // the timeout happens before the read, we'll skip over a pending byte to see if that
            // lets us find a valid frame.
            match future::select(read_fut, timeout).await {
                Either::Left((res, timeout)) => {
                    // The read finished! Return the bytes read.
                    let bytes_read = res?;

                    // Preserve the timer since we don't know if we parsed any frames yet.
                    parse_frame_timeout = Some(timeout);

                    Some(bytes_read)
                }
                Either::Right(((), _)) => {
                    // We timed out, so return None to signal we need to skip the next unparsed byte.
                    None
                }
            }
        } else {
            Some(read_fut.await.context("serial.read failed")?)
        };

        let deframe_time = std::time::Instant::now();

        // If we received any bytes, try to parse them. Otherwise skip any pending bytes to see if
        // that will let us parse a frame.
        //
        // Note: we intentionally try to parse frames before we check if the incoming channel was
        // closed (signified by reading zero bytes). This will allow the deframer to pass along any
        // other pending bytes as unframed data.
        let frames = if let Some(bytes_read) = bytes_read {
            tracing::trace!("SERIAL GOT BYTES: {:?}", &buf[..bytes_read]);

            deframer.parse_frames(&buf[..bytes_read])
        } else {
            tracing::trace!("SERIAL TIMED OUT");

            deframer.skip_byte_and_parse_frames()
        };

        // Send along any frames we parsed.
        let mut got_frames = false;
        for frame in frames {
            got_frames = true;
            deframer_writer.feed(frame?).await?;
        }
        deframer_writer.flush().await?;

        if let Some(bytes_read) = bytes_read {
            tracing::trace!(
                "SERIAL QUEUED BYTES after {:?}: {:?}",
                std::time::Instant::now() - deframe_time,
                &buf[..bytes_read]
            );
        }

        // Exit if the incoming stream was closed.
        if bytes_read == Some(0) {
            return Ok(());
        }

        // Clear the timer if we don't have any pending bytes. Otherwise, reset the timer if we
        // parsed at least one frame.
        if !deframer.has_pending_bytes() {
            parse_frame_timeout = None;
        } else if got_frames {
            parse_frame_timeout = Some(Timer::new(INCOMING_BYTE_TIMEOUT));
        }
    }
}

async fn reset<OutputSink: AsyncWrite + Unpin>(
    role: Role,
    mut fragment_reader: StreamSplitter<OutputSink>,
    mut fragment_writer: FragmentWriter,
) -> Result<(), Error> {
    tracing::trace!("RESET SERIAL BEGIN");
    let drain_time = match role {
        Role::Client => Duration::from_secs(3),
        Role::Server => Duration::from_secs(3),
    };
    enum DrainError {
        FromRead(Error),
        Timeout,
    }
    loop {
        match fragment_reader
            .read()
            .map_err(DrainError::FromRead)
            .on_timeout(drain_time, || Err(DrainError::Timeout))
            .await
        {
            Err(DrainError::Timeout) => break,
            Ok(frame) => {
                eprintln!("discard frame during drain: {:?}", frame);
                fragment_writer.write(vec![]).await?;
                continue;
            }
            Err(DrainError::FromRead(e)) => return Err(e),
        }
    }
    tracing::trace!("RESET SERIAL END");
    Ok(())
}

async fn link_to_framer(
    mut link_sender: LinkSender,
    mut fragment_writer: FragmentWriter,
) -> Result<(), Error> {
    while let Some(frame) = link_sender.next_send().await {
        let send = frame.bytes().to_vec();
        drop(frame);
        fragment_writer.write(send).await?;
    }
    Ok(())
}

async fn deframer_to_link<OutputSink: AsyncWrite + Unpin>(
    role: Role,
    mut fragment_reader: StreamSplitter<OutputSink>,
    mut link_receiver: LinkReceiver,
) -> Result<(), Error> {
    let mut know_peer_id = false;
    loop {
        let mut frame = fragment_reader.read().await?;
        tracing::trace!("READ FRAME: {:?}", frame);
        if frame.is_empty() {
            return Err(format_err!("reset received"));
        }
        link_receiver.received_frame(frame.as_mut()).await;
        if !know_peer_id {
            if let Some(peer_node_id) = link_receiver.peer_node_id() {
                // This log line is load bearing to the Overnet serial test.
                tracing::info!(
                    "Established {:?} Overnet serial connection to peer {:?}",
                    role,
                    peer_node_id
                );
                know_peer_id = true;
            }
        }
    }
}

async fn main<OutputSink: AsyncWrite + Unpin>(
    role: Role,
    fragment_reader: StreamSplitter<OutputSink>,
    fragment_writer: FragmentWriter,
    router: &WeakRouter,
    descriptor: Option<String>,
) -> Result<(), Error> {
    let (link_sender, link_receiver) = router.get()?.new_link(
        Default::default(),
        Box::new(move || {
            descriptor.clone().map(|d| match role {
                Role::Server => fidl_fuchsia_overnet_protocol::LinkConfig::SerialServer(d),
                Role::Client => fidl_fuchsia_overnet_protocol::LinkConfig::SerialClient(d),
            })
        }),
    );
    futures::future::try_join(
        link_to_framer(link_sender, fragment_writer),
        deframer_to_link(role, fragment_reader, link_receiver),
    )
    .map_ok(drop)
    .await
}

#[cfg(test)]
mod tests {
    use super::Role;
    use crate::report_skipped::ReportSkipped;
    use anyhow::format_err;
    use fuchsia_async::{Task, TimeoutExt};
    use futures::prelude::*;
    use onet_test_util::{test_security_context, DodgyPipe};
    use overnet_core::{NodeId, Router, RouterOptions};
    use std::sync::Arc;
    use std::time::Duration;

    async fn await_peer(router: Arc<Router>, peer: NodeId) {
        let lp = router.new_list_peers_context();
        while lp.list_peers().await.unwrap().into_iter().find(|p| peer == p.id.into()).is_none() {}
    }

    async fn end2end(run: usize, failures_per_64kib: u16) {
        let _ = tracing_subscriber::fmt::try_init();

        let rtr_client = Router::new(
            RouterOptions::new().set_node_id((100 * (run as u64) + 1).into()),
            test_security_context(),
        )
        .unwrap();

        let rtr_server = Router::new(
            RouterOptions::new().set_node_id((100 * (run as u64) + 2).into()),
            test_security_context(),
        )
        .unwrap();

        let (c2s_rx, c2s_tx) = DodgyPipe::new(failures_per_64kib).split();
        let (s2c_rx, s2c_tx) = DodgyPipe::new(failures_per_64kib).split();

        let run_client = super::run(
            Role::Client,
            s2c_rx,
            c2s_tx,
            Arc::downgrade(&rtr_client),
            ReportSkipped::new("client"),
            None,
        );

        let run_server = super::run(
            Role::Server,
            c2s_rx,
            s2c_tx,
            Arc::downgrade(&rtr_server),
            ReportSkipped::new("server"),
            None,
        );

        let _fwd = Task::spawn(
            futures::future::join(
                async move { panic!("should never terminate: {:?}", run_client.await) },
                async move { panic!("should never terminate: {:?}", run_server.await) },
            )
            .map(drop),
        );

        futures::future::join(
            await_peer(rtr_client.clone(), rtr_server.node_id()),
            await_peer(rtr_server.clone(), rtr_client.node_id()),
        )
        .map(Ok)
        .on_timeout(Duration::from_secs(120), || Err(format_err!("timeout")))
        .await
        .unwrap();
    }

    #[fuchsia_async::run_singlethreaded(test)]
    async fn reliable(run: usize) {
        end2end(run, 0).await
    }

    #[fuchsia_async::run_singlethreaded(test)]
    async fn mostly_reliable(run: usize) {
        end2end(run, 1).await
    }
}
