// Copyright 2018 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#![recursion_limit = "512"]

use {
    anyhow::{format_err, Context as _, Error},
    async_helpers::hanging_get::asynchronous as hanging_get,
    fidl::endpoints::ServiceMarker,
    fidl_fuchsia_bluetooth::Appearance,
    fidl_fuchsia_bluetooth_bredr::ProfileMarker,
    fidl_fuchsia_bluetooth_control::ControlRequestStream,
    fidl_fuchsia_bluetooth_gatt::Server_Marker,
    fidl_fuchsia_bluetooth_le::{CentralMarker, PeripheralMarker},
    fidl_fuchsia_device::{NameProviderMarker, DEFAULT_DEVICE_NAME},
    fidl_fuchsia_sys::ComponentControllerEvent,
    fuchsia_async as fasync,
    fuchsia_component::{
        client::{self, connect_to_service, App},
        fuchsia_single_component_package_url,
        server::{NestedEnvironment, ServiceFs},
    },
    fuchsia_zircon as zx,
    futures::{channel::mpsc, future::try_join5, FutureExt, StreamExt, TryFutureExt, TryStreamExt},
    log::{error, info, warn},
    pin_utils::pin_mut,
    std::collections::HashMap,
};

use crate::{
    devices::{watch_hosts, HostEvent::*},
    generic_access_service::GenericAccessService,
    host_dispatcher::{HostService::*, *},
    services::host_watcher,
    watch_peers::PeerWatcher,
};

mod build_config;
mod devices;
mod generic_access_service;
mod host_device;
mod host_dispatcher;
mod services;
mod store;
#[cfg(test)]
mod test;
mod types;
mod watch_peers;

const BT_GAP_COMPONENT_ID: &'static str = "bt-gap";

#[fasync::run_singlethreaded]
async fn main() -> Result<(), Error> {
    fuchsia_syslog::init_with_tags(&["bt-gap"]).expect("Can't init logger");
    info!("Starting bt-gap...");
    let result = run().await.context("Error running BT-GAP");
    if let Err(e) = &result {
        error!("{:?}", e)
    };
    Ok(result?)
}

// Returns the device host name that we assign as the local Bluetooth device name by default.
async fn get_host_name() -> Result<String, Error> {
    // Obtain the local device name to assign it as the default Bluetooth name,
    let name_provider = connect_to_service::<NameProviderMarker>()?;
    name_provider
        .get_device_name()
        .await?
        .map_err(|e| format_err!("failed to obtain host name: {:?}", e))
}

/// Creates a NestedEnvironment and launches the component specified by `component_url`. The
/// provided component must provide the Profile service.
/// Returns the controller for the launched component and the NestedEnvironment - dropping
/// either will result in component termination.
async fn launch_profile_forwarding_component(
    component_url: String,
    profile_hd: HostDispatcher,
) -> Result<(App, NestedEnvironment), Error> {
    let mut rfcomm_fs = ServiceFs::new();
    rfcomm_fs
        .add_service_at(ProfileMarker::NAME, move |chan| {
            info!("Connecting Profile Service to Host Device");
            fasync::Task::spawn(profile_hd.clone().request_host_service(chan, Profile)).detach();
            None
        })
        .add_proxy_service::<fidl_fuchsia_logger::LogSinkMarker, _>()
        .add_proxy_service::<fidl_fuchsia_cobalt::LoggerFactoryMarker, _>();

    let env = rfcomm_fs.create_salted_nested_environment("bt-rfcomm")?;
    fasync::Task::spawn(rfcomm_fs.collect()).detach();

    let bt_rfcomm = client::launch(&env.launcher(), component_url, None)?;

    // Wait until component has successfully launched before returning the controller.
    let mut event_stream = bt_rfcomm.controller().take_event_stream();
    match event_stream.next().await {
        Some(Ok(ComponentControllerEvent::OnDirectoryReady { .. })) => Ok((bt_rfcomm, env)),
        x => Err(format_err!("Launch failure: {:?}", x)),
    }
}

/// Creates a handler that will setup an incoming Profile service channel.
///
/// If `component` is set, the channel will be relayed to the component.
/// Otherwise, it will be relayed directly to the dispatcher.
fn profile_handler(
    dispatcher: &HostDispatcher,
    component: Option<(App, NestedEnvironment)>,
) -> impl FnMut(zx::Channel) -> Option<()> {
    let hd = dispatcher.clone();
    move |chan| {
        match &component {
            Some((rfcomm_component, _env)) => {
                info!("Passing Profile to RFCOMM Service");
                let _ = rfcomm_component.pass_to_service::<ProfileMarker>(chan);
            }
            None => {
                info!("Directly connecting Profile Service to Host Device");
                fasync::Task::spawn(hd.clone().request_host_service(chan, Profile)).detach();
            }
        }
        None
    }
}

fn host_service_handler(
    dispatcher: &HostDispatcher,
    service_name: &'static str,
    service: HostService,
) -> impl FnMut(fuchsia_zircon::Channel) -> Option<()> {
    let dispatcher = dispatcher.clone();
    move |chan| {
        info!("Connecting {} to Host Device", service_name);
        fasync::Task::spawn(dispatcher.clone().request_host_service(chan, service)).detach();
        None
    }
}

async fn run() -> Result<(), Error> {
    let inspect = fuchsia_inspect::Inspector::new();
    let stash_inspect = inspect.root().create_child("persistent");
    let stash = store::stash::init_stash(BT_GAP_COMPONENT_ID, stash_inspect)
        .await
        .context("Error initializing Stash service")?;

    let local_name = get_host_name().await.unwrap_or(DEFAULT_DEVICE_NAME.to_string());
    let (gas_channel_sender, generic_access_req_stream) = mpsc::channel(0);

    // Initialize a HangingGetBroker to process watch_peers requests
    let watch_peers_broker = hanging_get::HangingGetBroker::new(
        HashMap::new(),
        PeerWatcher::observe,
        hanging_get::DEFAULT_CHANNEL_SIZE,
    );
    let watch_peers_publisher = watch_peers_broker.new_publisher();
    let watch_peers_registrar = watch_peers_broker.new_registrar();

    // Initialize a HangingGetBroker to process watch_hosts requests
    let watch_hosts_broker = hanging_get::HangingGetBroker::new(
        Vec::new(),
        host_watcher::observe_hosts,
        hanging_get::DEFAULT_CHANNEL_SIZE,
    );
    let watch_hosts_publisher = watch_hosts_broker.new_publisher();
    let watch_hosts_registrar = watch_hosts_broker.new_registrar();

    // Process the watch_peers broker in the background
    let run_watch_peers = watch_peers_broker
        .run()
        .map(|()| Err::<(), Error>(format_err!("WatchPeers broker terminated unexpectedly")));
    // Process the watch_hosts broker in the background
    let run_watch_hosts = watch_hosts_broker
        .run()
        .map(|()| Err::<(), Error>(format_err!("WatchHosts broker terminated unexpectedly")));

    let hd = HostDispatcher::new(
        local_name,
        Appearance::Display,
        stash,
        inspect.root().create_child("system"),
        gas_channel_sender,
        watch_peers_publisher,
        watch_peers_registrar,
        watch_hosts_publisher,
        watch_hosts_registrar,
    );
    let watch_hd = hd.clone();
    let bootstrap_hd = hd.clone();
    let access_hd = hd.clone();
    let config_hd = hd.clone();
    let hostwatcher_hd = hd.clone();

    let host_watcher_task = async {
        let stream = watch_hosts();
        pin_mut!(stream);
        while let Some(msg) = stream.try_next().await? {
            match msg {
                HostAdded(device_path) => {
                    let result = watch_hd.add_device(&device_path).await;
                    if let Err(e) = &result {
                        warn!("Error adding bt-host device '{:?}': {:?}", device_path, e);
                    }
                    result?
                }
                HostRemoved(device_path) => {
                    watch_hd.rm_device(&device_path).await;
                }
            }
        }
        Ok(())
    };

    let generic_access_service_task =
        GenericAccessService { hd: hd.clone(), generic_access_req_stream }.run().map(|()| Ok(()));

    // Attempt to launch the RFCOMM component that will serve requests over `Profile` and forward
    // to the Host Server. If RFCOMM is not available for any reason, bt-gap will forward `Profile`
    // requests directly to the Host Server.
    let rfcomm_url = fuchsia_single_component_package_url!("bt-rfcomm").to_string();
    let bt_rfcomm = match launch_profile_forwarding_component(rfcomm_url, hd.clone()).await {
        Ok((rfcomm, env)) => Some((rfcomm, env)),
        Err(e) => {
            warn!("bt-gap unable to launch bt-rfcomm: {:?}", e);
            None
        }
    };

    let mut fs = ServiceFs::new();

    // serve bt-gap inspect VMO
    inspect.serve(&mut fs)?;

    fs.dir("svc")
        .add_fidl_service({
            let hd = hd.clone();
            move |s| control_service(hd.clone(), s)
        })
        .add_service_at(
            CentralMarker::NAME,
            host_service_handler(&hd, CentralMarker::DEBUG_NAME, LeCentral),
        )
        .add_service_at(
            PeripheralMarker::NAME,
            host_service_handler(&hd, PeripheralMarker::DEBUG_NAME, LePeripheral),
        )
        .add_service_at(
            Server_Marker::NAME,
            host_service_handler(&hd, Server_Marker::DEBUG_NAME, LeGatt),
        )
        .add_service_at(ProfileMarker::NAME, profile_handler(&hd, bt_rfcomm))
        // TODO(fxbug.dev/1496) - according fuchsia.bluetooth.sys/bootstrap.fidl, the bootstrap service should
        // only be available before initialization, and only allow a single commit before becoming
        // unservicable. This behavior interacts with parts of Bluetooth lifecycle and component
        // framework design that are not yet complete. For now, we provide the service to whomever
        // asks, whenever, but clients should not rely on this. The implementation will change once
        // we have a better solution.
        .add_fidl_service(move |request_stream| {
            info!("Serving Bootstrap Service");
            fasync::Task::spawn(
                services::bootstrap::run(bootstrap_hd.clone(), request_stream)
                    .unwrap_or_else(|e| warn!("Bootstrap service failed: {:?}", e)),
            )
            .detach();
        })
        .add_fidl_service(move |request_stream| {
            info!("Serving Access Service");
            fasync::Task::spawn(
                services::access::run(access_hd.clone(), request_stream)
                    .unwrap_or_else(|e| warn!("Access service failed: {:?}", e)),
            )
            .detach();
        })
        .add_fidl_service(move |request_stream| {
            info!("Serving Configuration Service");
            fasync::Task::spawn(
                services::configuration::run(config_hd.clone(), request_stream)
                    .unwrap_or_else(|e| warn!("Configuration service failed: {:?}", e)),
            )
            .detach();
        })
        .add_fidl_service(move |request_stream| {
            info!("Serving HostWatcher Service");
            fasync::Task::spawn(
                services::host_watcher::run(hostwatcher_hd.clone(), request_stream)
                    .unwrap_or_else(|e| warn!("HostWatcher service failed: {:?}", e)),
            )
            .detach();
        });
    fs.take_and_serve_directory_handle()?;
    let fs_task = fs.collect::<()>().map(Ok);

    try_join5(
        fs_task,
        host_watcher_task,
        generic_access_service_task,
        run_watch_peers,
        run_watch_hosts,
    )
    .await
    .map(|_| ())
}

fn control_service(hd: HostDispatcher, stream: ControlRequestStream) {
    info!("Spawning Control Service");
    fasync::Task::spawn(
        services::start_control_service(hd.clone(), stream)
            .unwrap_or_else(|e| eprintln!("Failed to spawn {:?}", e)),
    )
    .detach()
}
