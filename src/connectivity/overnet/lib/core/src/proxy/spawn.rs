// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Factory functions for proxies - one each for sending a handle and receiving a handle.

use super::{main, Proxy, ProxyTransferInitiationReceiver};
use crate::async_quic::{AsyncConnection, StreamProperties};
use crate::framed_stream::{FramedStreamReader, FramedStreamWriter, MessageStats};
use crate::proxyable_handle::{IntoProxied, Proxyable, ProxyableHandle};
use crate::router::{FoundTransfer, OpenedTransfer, Router};
use anyhow::{format_err, Error};
use fidl_fuchsia_overnet_protocol::{StreamId, StreamRef, TransferInitiator, TransferWaiter};
use std::rc::{Rc, Weak};

pub(crate) fn send<Hdl: 'static + Proxyable>(
    hdl: Hdl,
    initiate_transfer: ProxyTransferInitiationReceiver,
    stream_writer: FramedStreamWriter,
    stream_reader: FramedStreamReader,
    stats: Rc<MessageStats>,
    router: Weak<Router>,
) -> Result<(), Error> {
    log::info!(
        "[PROXY {:?}] spawn from {:?} to {:?}:{:?} for send",
        hdl,
        Weak::upgrade(&router).map(|r| r.node_id()),
        stream_writer.peer_node_id(),
        stream_writer.id()
    );

    main::spawn_main_loop(
        Proxy::new(hdl, router, stats),
        initiate_transfer,
        stream_writer,
        None,
        stream_reader,
    )
}

// Start receiving from some stream for a channel.
// Returns a handle, and a boolean indicating whether proxying is actually occuring (true) or not.
pub(crate) async fn recv<Hdl: 'static + Proxyable, CreateType>(
    create_handles: impl FnOnce() -> Result<(CreateType, CreateType), Error>,
    initiate_transfer: ProxyTransferInitiationReceiver,
    stream_ref: StreamRef,
    conn: &AsyncConnection,
    stats: Rc<MessageStats>,
    router: Weak<Router>,
) -> Result<(fidl::Handle, bool), Error>
where
    CreateType: fidl::HandleBased + IntoProxied<Proxied = Hdl> + std::fmt::Debug,
{
    Ok(match stream_ref {
        StreamRef::Creating(StreamId { id: stream_id }) => {
            let (app_chan, overnet_chan) = create_handles()?;
            let (stream_writer, stream_reader) = conn.bind_id(stream_id);
            let overnet_chan = overnet_chan.into_proxied()?;
            log::info!(
                "[PROXY {:?}] spawn from {:?}:{:?} to {:?} for recv with pair {:?}",
                overnet_chan,
                stream_writer.peer_node_id(),
                stream_writer.id(),
                Weak::upgrade(&router).map(|r| r.node_id()),
                app_chan
            );
            super::main::spawn_main_loop(
                Proxy::new(overnet_chan, router, stats),
                initiate_transfer,
                stream_writer.into(),
                None,
                stream_reader.into(),
            )?;
            (app_chan.into_handle(), true)
        }
        StreamRef::TransferInitiator(TransferInitiator {
            stream_id: StreamId { id: stream_id },
            new_destination_node,
            transfer_key,
        }) => {
            let (app_chan, overnet_chan) = create_handles()?;
            let initial_stream_reader: FramedStreamReader = conn.bind_uni_id(stream_id).into();
            // TODO good link hint
            let opened_transfer = Weak::upgrade(&router)
                .ok_or_else(|| format_err!("No router to handle draining stream ref"))?
                .open_transfer(
                    new_destination_node.into(),
                    transfer_key,
                    overnet_chan.into_handle(),
                    &Weak::new(),
                )
                .await?;
            match opened_transfer {
                OpenedTransfer::Fused => {
                    let app_chan = app_chan.into_proxied()?;
                    log::info!(
                        "[PROXY] fuse drain {:?}:{:?} into {:?} handle {:?} after opening {:?}",
                        initial_stream_reader.peer_node_id(),
                        initial_stream_reader.id(),
                        Weak::upgrade(&router).map(|r| r.node_id()),
                        app_chan,
                        transfer_key
                    );
                    (
                        ProxyableHandle::new(app_chan, router, stats)
                            .drain_stream_to_handle(initial_stream_reader)
                            .await?,
                        false,
                    )
                }
                OpenedTransfer::Remote(stream_writer, stream_reader, overnet_chan) => {
                    log::info!(
                        "[PROXY {:?}] spawn from {:?}:{:?} then {:?}:{:?} to {:?} for recv with pair {:?} after opening {:?}",
                        overnet_chan,
                        initial_stream_reader.peer_node_id(),
                        initial_stream_reader.id(),
                        stream_writer.peer_node_id(),
                        stream_writer.id(),
                        Weak::upgrade(&router).map(|r| r.node_id()),
                        app_chan,
                        transfer_key
                    );
                    main::spawn_main_loop(
                        Proxy::new(Hdl::from_fidl_handle(overnet_chan)?, router, stats),
                        initiate_transfer,
                        stream_writer.into(),
                        Some(initial_stream_reader),
                        stream_reader.into(),
                    )?;
                    (app_chan.into_handle(), true)
                }
            }
        }
        StreamRef::TransferWaiter(TransferWaiter {
            stream_id: StreamId { id: stream_id },
            transfer_key,
        }) => {
            let initial_stream_reader: FramedStreamReader = conn.bind_uni_id(stream_id).into();
            let found_transfer = Weak::upgrade(&router)
                .ok_or_else(|| format_err!("No router to handle draining stream ref"))?
                .find_transfer(transfer_key)
                .await?;
            match found_transfer {
                FoundTransfer::Fused(handle) => {
                    let handle = Hdl::from_fidl_handle(handle)?;
                    log::info!(
                        "[PROXY] fuse drain {:?}:{:?} into {:?} handle {:?} after waiting for {:?}",
                        initial_stream_reader.peer_node_id(),
                        initial_stream_reader.id(),
                        Weak::upgrade(&router).map(|r| r.node_id()),
                        handle,
                        transfer_key
                    );
                    (
                        ProxyableHandle::new(handle, router, stats)
                            .drain_stream_to_handle(initial_stream_reader.into())
                            .await?,
                        false,
                    )
                }
                FoundTransfer::Remote(stream_writer, stream_reader) => {
                    let (app_chan, overnet_chan) = create_handles()?;
                    log::info!(
                        "[PROXY {:?}] spawn from {:?}:{:?} then {:?}:{:?} to {:?} for recv with pair {:?} after waiting for {:?}",
                        overnet_chan,
                        initial_stream_reader.peer_node_id(),
                        initial_stream_reader.id(),
                        stream_writer.peer_node_id(),
                        stream_writer.id(),
                        Weak::upgrade(&router).map(|r| r.node_id()),
                        app_chan,
                        transfer_key
                    );
                    main::spawn_main_loop(
                        Proxy::new(overnet_chan.into_proxied()?, router, stats),
                        initiate_transfer,
                        stream_writer.into(),
                        Some(initial_stream_reader),
                        stream_reader.into(),
                    )?;
                    (app_chan.into_handle(), true)
                }
            }
        }
    })
}
