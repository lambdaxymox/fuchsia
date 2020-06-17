// Copyright 2020 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::error::PowerManagerError;
use crate::message::{Message, MessageReturn};
use crate::node::Node;
use crate::shutdown_request::ShutdownRequest;
use crate::types::Seconds;
use crate::utils::connect_proxy;
use anyhow::{format_err, Error};
use async_trait::async_trait;
use fidl_fuchsia_hardware_power_statecontrol as fpowercontrol;
use fidl_fuchsia_sys2 as fsys;
use fuchsia_async::{self as fasync, DurationExt, TimeoutExt};
use fuchsia_component::server::{ServiceFs, ServiceObjLocal};
use fuchsia_inspect::{self as inspect, Property};
use fuchsia_zircon::{self as zx, Status as zx_status};
use futures::prelude::*;
use futures::TryStreamExt;
use log::*;
use serde_derive::Deserialize;
use serde_json as json;
use std::cell::Cell;
use std::collections::HashMap;
use std::convert::TryInto;
use std::rc::Rc;

/// Node: SystemShutdownHandler
///
/// Summary: Provides a mechanism for the Power Manager to shut down the system. In addition to
/// providing a shutdown mechanism for the Power Manager internally, this node hosts the
/// fuchsia.hardware.power.statecontrol protocol to support system shutdown for external components.
///
/// Handles Messages:
///     - SystemShutdown
///
/// Sends Messages:
///     - SetTerminationSystemState
///
/// FIDL dependencies:
///     - fuchsia.device.manager: this FIDL library provides the SystemPowerState enum that the
///       Driver Manager expects as an argument to its fuchsia.device.manager.SystemStateTransition
///       API.
///     - fuchsia.hardware.power.statecontrol.Admin: the node hosts a service of this protocol to
///       the rest of the system
///     - fuchsia.sys2.SystemController: the node uses this protocol to instruct the Component
///       Manager to shut down the component tree

/// A builder for constructing the SystemShutdownHandler node.
pub struct SystemShutdownHandlerBuilder<'a, 'b> {
    driver_manager_handler: Rc<dyn Node>,
    component_mgr_path: Option<String>,
    component_mgr_proxy: Option<fsys::SystemControllerProxy>,
    shutdown_timeout: Option<Seconds>,
    force_shutdown_func: Box<dyn Fn()>,
    service_fs: Option<&'a mut ServiceFs<ServiceObjLocal<'b, ()>>>,
    inspect_root: Option<&'a inspect::Node>,
}

impl<'a, 'b> SystemShutdownHandlerBuilder<'a, 'b> {
    pub fn new(driver_manager_handler: Rc<dyn Node>) -> Self {
        Self {
            driver_manager_handler,
            component_mgr_path: None,
            component_mgr_proxy: None,
            shutdown_timeout: None,
            force_shutdown_func: Box::new(force_shutdown),
            service_fs: None,
            inspect_root: None,
        }
    }

    pub fn new_from_json(
        json_data: json::Value,
        nodes: &HashMap<String, Rc<dyn Node>>,
        service_fs: &'a mut ServiceFs<ServiceObjLocal<'b, ()>>,
    ) -> Self {
        #[derive(Deserialize)]
        struct Config {
            service_path: String,
            shutdown_timeout: Option<f64>,
        };

        #[derive(Deserialize)]
        struct Dependencies {
            driver_manager_handler_node: String,
        }

        #[derive(Deserialize)]
        struct JsonData {
            config: Config,
            dependencies: Dependencies,
        };

        let data: JsonData = json::from_value(json_data).unwrap();
        let mut builder = Self::new(nodes[&data.dependencies.driver_manager_handler_node].clone())
            .with_service_fs(service_fs)
            .with_component_mgr_path(data.config.service_path);

        if data.config.shutdown_timeout.is_some() {
            builder = builder.with_shutdown_timeout(Seconds(data.config.shutdown_timeout.unwrap()))
        }

        builder
    }

    pub fn with_component_mgr_path(mut self, path: String) -> Self {
        self.component_mgr_path = Some(path);
        self
    }

    pub fn with_service_fs(
        mut self,
        service_fs: &'a mut ServiceFs<ServiceObjLocal<'b, ()>>,
    ) -> Self {
        self.service_fs = Some(service_fs);
        self
    }

    pub fn with_shutdown_timeout(mut self, timeout: Seconds) -> Self {
        self.shutdown_timeout = Some(timeout);
        self
    }

    #[cfg(test)]
    pub fn with_force_shutdown_function(
        mut self,
        force_shutdown: Box<impl Fn() + 'static>,
    ) -> Self {
        self.force_shutdown_func = force_shutdown;
        self
    }

    #[cfg(test)]
    pub fn with_component_mgr_proxy(mut self, proxy: fsys::SystemControllerProxy) -> Self {
        self.component_mgr_proxy = Some(proxy);
        self
    }

    #[cfg(test)]
    pub fn with_inspect_root(mut self, root: &'a inspect::Node) -> Self {
        self.inspect_root = Some(root);
        self
    }

    pub fn build(self) -> Result<Rc<SystemShutdownHandler>, Error> {
        // Optionally use the default inspect root node
        let inspect_root = self.inspect_root.unwrap_or(inspect::component::inspector().root());

        // Connect to the Component Manager's SystemController service if a proxy wasn't specified
        let component_mgr_proxy = if let Some(proxy) = self.component_mgr_proxy {
            proxy
        } else {
            connect_proxy::<fsys::SystemControllerMarker>(
                &self
                    .component_mgr_path
                    .ok_or(format_err!("Must specify Component Manager path or proxy"))?
                    .to_string(),
            )?
        };

        let node = Rc::new(SystemShutdownHandler {
            shutdown_timeout: self.shutdown_timeout,
            force_shutdown_func: self.force_shutdown_func,
            shutdown_pending: Cell::new(false),
            driver_mgr_handler: self.driver_manager_handler,
            component_mgr_proxy,
            inspect: InspectData::new(inspect_root, "SystemShutdownHandler".to_string()),
        });

        // Publish the service only if we were provided with a ServiceFs
        if self.service_fs.is_some() {
            node.clone().publish_fidl_service(self.service_fs.unwrap());
        }

        // Populate the config data into Inspect
        node.inspect.set_config(&node);

        Ok(node)
    }
}

pub struct SystemShutdownHandler {
    /// Maximum time to wait for an orderly shutdown using the Component Manager shutdown path. If
    /// the timeout expires before the orderly shutdown path is complete, a forced shutdown will
    /// occur. If the orderly shutdown path fails with an error, a forced shutdown will occur
    /// immediately. If no shutdown timeout is specified, a forced shutdown will occur immediately
    /// following the failed attempt.
    shutdown_timeout: Option<Seconds>,

    /// Function to force a system shutdown.
    force_shutdown_func: Box<dyn Fn()>,

    /// Tracks the current shutdown request state. Used to ignore shutdown requests while a current
    /// request is being processed.
    shutdown_pending: Cell<bool>,

    /// Reference to the DriverManagerHandler node. It is expected that this node responds to the
    /// SetTerminationSystemState message.
    driver_mgr_handler: Rc<dyn Node>,

    /// Proxy handle to communicate with the Component Manager's SystemController protocol.
    component_mgr_proxy: fsys::SystemControllerProxy,

    /// Struct for managing Component Inspection data
    inspect: InspectData,
}

impl SystemShutdownHandler {
    /// Start and publish the fuchsia.hardware.power.statecontrol.Admin service
    fn publish_fidl_service<'a, 'b>(
        self: Rc<Self>,
        service_fs: &'a mut ServiceFs<ServiceObjLocal<'b, ()>>,
    ) {
        service_fs.dir("svc").add_fidl_service(move |stream| {
            self.clone().handle_new_service_connection(stream);
        });
    }

    /// Called each time a client connects. For each client, a future is created to handle the
    /// request stream.
    fn handle_new_service_connection(
        self: Rc<Self>,
        mut stream: fpowercontrol::AdminRequestStream,
    ) {
        fuchsia_trace::instant!(
            "power_manager",
            "SystemShutdownHandler::handle_new_service_connection",
            fuchsia_trace::Scope::Thread
        );
        fasync::spawn_local(
            async move {
                while let Some(req) = stream.try_next().await? {
                    match req {
                        fpowercontrol::AdminRequest::Suspend { state, responder } => {
                            let result = self.handle_shutdown(state.try_into()?).await;
                            let _ = responder.send(&mut result.map_err(|e| e.into_raw()));
                        }
                        fpowercontrol::AdminRequest::PowerFullyOn { responder } => {
                            let _ = responder.send(&mut Err(zx::Status::NOT_SUPPORTED.into_raw()));
                        }
                        fpowercontrol::AdminRequest::Reboot { reason, responder } => {
                            let result =
                                self.handle_shutdown(ShutdownRequest::Reboot(reason)).await;
                            let _ = responder.send(&mut result.map_err(|e| e.into_raw()));
                        }
                        fpowercontrol::AdminRequest::RebootToBootloader { responder } => {
                            let result =
                                self.handle_shutdown(ShutdownRequest::RebootBootloader).await;
                            let _ = responder.send(&mut result.map_err(|e| e.into_raw()));
                        }
                        fpowercontrol::AdminRequest::RebootToRecovery { responder } => {
                            let result =
                                self.handle_shutdown(ShutdownRequest::RebootRecovery).await;
                            let _ = responder.send(&mut result.map_err(|e| e.into_raw()));
                        }
                        fpowercontrol::AdminRequest::Poweroff { responder } => {
                            let result = self.handle_shutdown(ShutdownRequest::PowerOff).await;
                            let _ = responder.send(&mut result.map_err(|e| e.into_raw()));
                        }
                        fpowercontrol::AdminRequest::Mexec { responder } => {
                            let result = self.handle_shutdown(ShutdownRequest::Mexec).await;
                            let _ = responder.send(&mut result.map_err(|e| e.into_raw()));
                        }
                        fpowercontrol::AdminRequest::SuspendToRam { responder } => {
                            let _ = responder.send(&mut Err(zx::Status::NOT_SUPPORTED.into_raw()));
                        }
                    }
                }
                Ok(())
            }
            .unwrap_or_else(|e: anyhow::Error| error!("{:?}", e)),
        );
    }

    /// Called each time a client calls the fuchsia.hardware.power.statecontrol.Admin API to
    /// initiate a system state transition. If the function is called while a shutdown is already in
    /// progress, then an error is returned. This is the only scenario where the function will
    /// return. In all other cases, the function does not return.
    async fn handle_shutdown(&self, request: ShutdownRequest) -> Result<(), zx_status> {
        fuchsia_trace::instant!(
            "power_manager",
            "SystemShutdownHandler::handle_shutdown",
            fuchsia_trace::Scope::Thread,
            "request" => format!("{:?}", request).as_str()
        );

        // Return if shutdown is already pending
        if self.shutdown_pending.replace(true) == true {
            return Err(zx_status::ALREADY_EXISTS);
        }

        info!("System shutdown ({:?})", request);
        self.inspect.log_shutdown_request(&request);

        // Handle the shutdown using a timeout if one is present in the config
        let result = if self.shutdown_timeout.is_some() {
            self.shutdown_with_timeout(request, self.shutdown_timeout.unwrap()).await
        } else {
            self.shutdown(request).await
        };

        // If the result is an error, either by underlying API failure or by timeout, then force a
        // shutdown using the configured force_shutdown_func
        if result.is_err() {
            self.inspect.force_shutdown_attempted.set(true);
            (self.force_shutdown_func)();
        }

        Ok(())
    }

    /// Wraps the node's `shutdown` function with a timeout value. If the `shutdown` call has not
    /// returned within the specified timeout, an error is returned. In the event of a timeout, the
    /// underlying channel that encountered the timeout may encounter issues with subsequent
    /// transactions (fxbug.dev/53760). In this case, that means either the channel used by the
    /// `driver_mgr_handler` node or the `component_mgr_proxy` channel encountered the timeout. In
    /// all likelihood, because Driver Manager will not be doing any meaningful work in response to
    /// this FIDL call, we can presume that Component Manager, which will be performing a
    /// significant amount of work, is the channel that encountered the timeout. Therefore, care
    /// should be taken to avoid reusing the `component_mgr_proxy` channel in the event of a
    /// shutdown timeout. Currently this is not a problem because in the event of a timeout, we will
    /// force a system shutdown by other means.
    async fn shutdown_with_timeout(
        &self,
        request: ShutdownRequest,
        timeout: Seconds,
    ) -> Result<(), Error> {
        let deadline = zx::Duration::from_seconds(timeout.0 as i64).after_now();
        self.shutdown(request).on_timeout(deadline, || Err(format_err!("Shutdown timed out"))).await
    }

    /// Shut down the system into the specified state. The function works as follows:
    ///     1. Update the Driver Manager's termination state to match the requested shutdown state
    ///     2. Issue a shutdown to the Component Manager
    ///     3. Once the Component Manager shutdown process reaches the Driver Manager component,
    ///        the system will be put into the state that is specified by the termination state
    async fn shutdown(&self, request: ShutdownRequest) -> Result<(), Error> {
        self.send_message(
            &self.driver_mgr_handler,
            &Message::SetTerminationSystemState(request.into()),
        )
        .await?;
        self.component_mgr_proxy.shutdown().await?;
        Ok(())
    }

    /// Handle a SystemShutdown message which is a request to shut down the system from within the
    /// PowerManager itself.
    async fn handle_system_shutdown_message(
        &self,
        request: ShutdownRequest,
    ) -> Result<MessageReturn, PowerManagerError> {
        match self.handle_shutdown(request).await {
            Ok(()) => Ok(MessageReturn::SystemShutdown),
            Err(zx_status::ALREADY_EXISTS) => {
                Err(PowerManagerError::Busy("Shutdown already in progress".to_string()))
            }
            Err(e) => Err(PowerManagerError::GenericError(format_err!("{}", e))),
        }
    }
}

/// Forcibly shuts down the system. The function works by exiting the power_manager process. Since
/// the power_manager is marked as a critical process to the root job, once the power_manager exits
/// the root job will also exit, and the system will reboot.
pub fn force_shutdown() {
    info!("Force shutdown requested");
    std::process::exit(1);
}

#[async_trait(?Send)]
impl Node for SystemShutdownHandler {
    fn name(&self) -> &'static str {
        "SystemShutdownHandler"
    }

    async fn handle_message(&self, msg: &Message) -> Result<MessageReturn, PowerManagerError> {
        match msg {
            Message::SystemShutdown(request) => self.handle_system_shutdown_message(*request).await,
            _ => Err(PowerManagerError::Unsupported),
        }
    }
}

struct InspectData {
    // Nodes
    root_node: inspect::Node,

    // Properties
    shutdown_request: inspect::StringProperty,
    force_shutdown_attempted: inspect::BoolProperty,
}

impl InspectData {
    fn new(parent: &inspect::Node, name: String) -> Self {
        let root_node = parent.create_child(name);
        Self {
            shutdown_request: root_node.create_string("shutdown_request", "None"),
            force_shutdown_attempted: root_node.create_bool("force_shutdown_attempted", false),
            root_node,
        }
    }

    /// Populates any configuration data from the SystemShutdownHandler into an Inspect config node.
    fn set_config(&self, handler: &SystemShutdownHandler) {
        let config_node = self.root_node.create_child("config");
        config_node.record_uint(
            "shutdown_timeout (s)",
            handler.shutdown_timeout.unwrap_or(Seconds(0.0)).0 as u64,
        );
        self.root_node.record(config_node);
    }

    /// Updates the `shutdown_request` property according to the provided request.
    fn log_shutdown_request(&self, request: &ShutdownRequest) {
        self.shutdown_request.set(format!("{:?}", request).as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shutdown_request::RebootReason;
    use crate::test::mock_node::{create_dummy_node, create_mock_node, MessageMatcher};
    use crate::{msg_eq, msg_ok_return};
    use inspect::assert_inspect_tree;
    use matches::assert_matches;

    /// Create a fake SystemController service proxy that responds to Shutdown requests by calling
    /// the provided closure.
    fn setup_fake_component_mgr_service(
        mut shutdown_function: impl FnMut() + 'static,
    ) -> fsys::SystemControllerProxy {
        let (proxy, mut stream) =
            fidl::endpoints::create_proxy_and_stream::<fsys::SystemControllerMarker>().unwrap();
        fasync::spawn_local(async move {
            while let Ok(req) = stream.try_next().await {
                match req {
                    Some(fsys::SystemControllerRequest::Shutdown { responder }) => {
                        shutdown_function();
                        let _ = responder.send();
                    }
                    e => panic!("Unexpected request: {:?}", e),
                }
            }
        });

        proxy
    }

    /// Tests that well-formed configuration JSON does not panic the `new_from_json` function.
    #[fasync::run_singlethreaded(test)]
    async fn test_new_from_json() {
        let json_data = json::json!({
            "type": "SystemShutdownHandler",
            "name": "system_shutdown_handler",
            "config": {
                "service_path": "/svc/fake"
            },
            "dependencies": {
                "driver_manager_handler_node": "dev_mgr"
            }
        });

        let mut nodes: HashMap<String, Rc<dyn Node>> = HashMap::new();
        nodes.insert("dev_mgr".to_string(), create_dummy_node());

        let _ = SystemShutdownHandlerBuilder::new_from_json(
            json_data,
            &nodes,
            &mut ServiceFs::new_local(),
        );
    }

    /// Tests for the presence and correctness of inspect data
    #[fasync::run_singlethreaded(test)]
    async fn test_inspect_data() {
        let inspector = inspect::Inspector::new();
        let node = SystemShutdownHandlerBuilder::new(create_dummy_node())
            .with_component_mgr_proxy(setup_fake_component_mgr_service(|| {}))
            .with_inspect_root(inspector.root())
            .with_force_shutdown_function(Box::new(|| {}))
            .with_shutdown_timeout(Seconds(100.0))
            .build()
            .unwrap();

        // Issue a shutdown call that will fail (because the DriverManager dummy node will respond
        // to SetTerminationSystemState with an error), which causes a force shutdown to be issued.
        // This gives us something interesting to verify in Inspect.
        let _ = node.handle_shutdown(ShutdownRequest::Reboot(RebootReason::HighTemperature)).await;

        assert_inspect_tree!(
            inspector,
            root: {
                SystemShutdownHandler: {
                    config: {
                        "shutdown_timeout (s)": 100u64
                    },
                    shutdown_request: "Reboot(HighTemperature)",
                    force_shutdown_attempted: true
                }
            }
        );
    }

    /// Tests that the `shutdown` function correctly sets the corresponding termination state on the
    /// Driver Manager and calls the Component Manager shutdown API.
    #[fasync::run_singlethreaded(test)]
    async fn test_shutdown() {
        // The test will call `shutdown` with each of these shutdown requests
        let shutdown_requests = vec![
            ShutdownRequest::Reboot(RebootReason::UserRequest),
            ShutdownRequest::RebootBootloader,
            ShutdownRequest::RebootRecovery,
            ShutdownRequest::PowerOff,
            ShutdownRequest::Mexec,
        ];

        // At the end of the test, verify the Component Manager's received shutdown count (expected
        // to be equal to the number of entries in the system_power_states vector)
        let shutdown_count = Rc::new(Cell::new(0));
        let shutdown_count_clone = shutdown_count.clone();

        // Create the mock Driver Manager node that expects to receive the SetTerminationSystemState
        // message for each state in `system_power_states`
        let driver_mgr_node = create_mock_node(
            "DriverMgrNode",
            shutdown_requests
                .iter()
                .map(|request| {
                    (
                        msg_eq!(SetTerminationSystemState((*request).into())),
                        msg_ok_return!(SetTerminationSystemState),
                    )
                })
                .collect(),
        );

        // Create the node with a special Component Manager proxy
        let node = SystemShutdownHandlerBuilder::new(driver_mgr_node)
            .with_component_mgr_proxy(setup_fake_component_mgr_service(move || {
                shutdown_count_clone.set(shutdown_count_clone.get() + 1);
            }))
            .build()
            .unwrap();

        // Call `suspend` for each power state. The mock Driver Manager node verifies that the
        // SetTerminationSystemState messages are sent as expected.
        for request in &shutdown_requests {
            let _ = node.shutdown(*request).await;
        }

        // Verify the Component Manager shutdown was called for each shutdown call
        assert_eq!(shutdown_count.get(), shutdown_requests.len());
    }

    /// Tests that the shutdown timer works as expected by returning with an error after the
    /// expected timeout period.
    #[test]
    fn test_shutdown_timeout() {
        // Need to use an Executor with fake time to test the timeout value
        let mut exec = fasync::Executor::new_with_fake_time().unwrap();
        exec.set_fake_time(fasync::Time::from_nanos(0));

        // Arbitrary shutdown request to be used in the test
        let shutdown_request = ShutdownRequest::PowerOff;

        // Arbitrary timeout value to be used in the test
        let timeout = Seconds(10.0);

        // Don't use `setup_fake_component_mgr_service` here because we want a stream object that
        // doesn't respond to the request. This way we can properly exercise the timeout path.
        let (proxy, _stream) =
            fidl::endpoints::create_proxy_and_stream::<fsys::SystemControllerMarker>().unwrap();

        // Create the SystemShutdownHandler node
        let node = SystemShutdownHandlerBuilder::new(create_mock_node(
            "DriverMgrNode",
            vec![(
                msg_eq!(SetTerminationSystemState(shutdown_request.into())),
                msg_ok_return!(SetTerminationSystemState),
            )],
        ))
        .with_component_mgr_proxy(proxy)
        .build()
        .unwrap();

        // Future that attempts a suspend with a timeout
        let mut shutdown_future = Box::pin(node.shutdown_with_timeout(shutdown_request, timeout));

        // Run the future until it stalls. In this case, the stall will happen because the fake
        // Component Manager proxy doesn't respond to the shutdown request.
        assert!(exec.run_until_stalled(&mut shutdown_future).is_pending());

        // Wake the timer and verify the timer was scheduled to fire at the expected time
        assert_eq!(exec.wake_next_timer(), Some(fasync::Time::from_nanos(timeout.into_nanos())));

        // Run the future again. This time it should complete with an error (due to the timeout)
        assert_matches!(
            exec.run_until_stalled(&mut shutdown_future),
            futures::task::Poll::Ready(Err(_))
        );
    }

    /// Tests that a second shutdown request will fail with an error if a prior request is already
    /// in progress.
    #[test]
    fn test_ignore_second_shutdown() {
        let mut exec = fasync::Executor::new().unwrap();

        // Arbitrary shutdown request to be used in the test
        let shutdown_request = ShutdownRequest::PowerOff;

        // Don't use `setup_fake_component_mgr_service` here because we want a stream object that
        // doesn't respond to the request. This way we can properly exercise the timeout path.
        let (proxy, _stream) =
            fidl::endpoints::create_proxy_and_stream::<fsys::SystemControllerMarker>().unwrap();
        let node = SystemShutdownHandlerBuilder::new(create_mock_node(
            "DriverMgrNode",
            vec![(
                msg_eq!(SetTerminationSystemState(shutdown_request.into())),
                msg_ok_return!(SetTerminationSystemState),
            )],
        ))
        .with_component_mgr_proxy(proxy)
        .build()
        .unwrap();

        // Run the first shutdown request. This is expected to stall because the fake Component
        // Manager proxy will not be responding to the request.
        assert!(exec
            .run_until_stalled(&mut Box::pin(node.handle_shutdown(shutdown_request)))
            .is_pending());

        // Run a second request and verify it returns with the expected error
        assert_matches!(
            exec.run_until_stalled(&mut Box::pin(node.handle_shutdown(shutdown_request))),
            futures::task::Poll::Ready(Err(zx_status::ALREADY_EXISTS))
        );
    }

    /// Tests that if an orderly shutdown request fails, the forced shutdown method is called.
    #[fasync::run_singlethreaded(test)]
    async fn test_force_shutdown() {
        let force_shutdown = Rc::new(Cell::new(false));
        let force_shutdown_clone = force_shutdown.clone();
        let force_shutdown_func = Box::new(move || {
            force_shutdown_clone.set(true);
        });

        let node = SystemShutdownHandlerBuilder::new(create_dummy_node())
            .with_component_mgr_proxy(setup_fake_component_mgr_service(|| {}))
            .with_force_shutdown_function(force_shutdown_func)
            .build()
            .unwrap();

        // Call the normal shutdown function. The orderly shutdown will fail because the
        // DriverManager dummy node will respond to the SetTerminationSystemState message with an
        // error. When the orderly shutdown fails, the forced shutdown method will be called.
        let _ = node.handle_shutdown(ShutdownRequest::PowerOff).await;
        assert_eq!(force_shutdown.get(), true);
    }
}
