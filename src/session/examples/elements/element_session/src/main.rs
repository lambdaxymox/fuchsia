// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use {
    anyhow::{Context as _, Error},
    element_management::{Element, ElementManager, ElementManagerError, SimpleElementManager},
    fidl::encoding::Decodable,
    fidl_fuchsia_session::{
        AnnotationError, Annotations, ElementControllerRequest, ElementControllerRequestStream,
        ElementManagerRequest, ElementManagerRequestStream, ProposeElementError,
    },
    fidl_fuchsia_session_examples::{ElementPingRequest, ElementPingRequestStream},
    fidl_fuchsia_sys2 as fsys, fuchsia_async as fasync,
    fuchsia_component::{client::connect_to_service, server::ServiceFs},
    futures::{StreamExt, TryStreamExt},
    rand::{distributions::Alphanumeric, thread_rng, Rng},
};

enum ExposedServices {
    ElementManager(ElementManagerRequestStream),
    ElementPing(ElementPingRequestStream),
}

// TODO(38577): Write example tests for the element session.

/// The child collection to add elements to. This must match a collection name declared in
/// this session's CML file.
const ELEMENT_COLLECTION_NAME: &str = "elements";

/// This session exposes one service which is offered to all elements started in the session and
/// prints a string when an element sends a request to said service.
///
/// It also exposes the [`fidl_fuchsia_session.ElementManager`] service which an element proposer
/// can connect to in order to add an element to the session.
#[fasync::run_singlethreaded]
async fn main() -> Result<(), Error> {
    let mut fs = ServiceFs::new_local();
    fs.dir("svc").add_fidl_service(ExposedServices::ElementPing);
    fs.dir("svc").add_fidl_service(ExposedServices::ElementManager);

    fs.take_and_serve_directory_handle()?;

    let realm =
        connect_to_service::<fsys::RealmMarker>().context("Could not connect to Realm service.")?;
    let element_manager = SimpleElementManager::new(realm);

    while let Some(service_request) = fs.next().await {
        match service_request {
            ExposedServices::ElementPing(request_stream) => {
                handle_element_ping_requests(request_stream)
                    .await
                    .expect("Failed to run element ping service.");
            }
            ExposedServices::ElementManager(request_stream) => {
                handle_element_manager_requests(request_stream, &element_manager)
                    .await
                    .expect("Failed to run element manager service.");
            }
        }
    }
    Ok(())
}

/// Handles the ping requests and prints to the terminal on success.
///
/// # Parameters
/// `stream`: The input channel which receives [`Ping`] requests.
///
/// # Returns
/// `Ok` if the service ran successfully, or an `Error` if execution halted unexpectedly.
async fn handle_element_ping_requests(mut stream: ElementPingRequestStream) -> Result<(), Error> {
    while let Some(ElementPingRequest::Ping { control_handle: _ }) =
        stream.try_next().await.context("Error handling ping request stream")?
    {
        println!("Element did ping session.");
    }
    Ok(())
}

/// Handles the [`ElementManager`] requests and launches the element session on success.
///
/// # Parameters
/// `stream`: The input channel which receives [`ElementManager`] requests.
///
/// # Returns
/// `Ok` if the element manager ran successfully, or an `ElementManagerError` if execution halted unexpectedly.
async fn handle_element_manager_requests(
    mut stream: ElementManagerRequestStream,
    element_manager: &impl ElementManager,
) -> Result<(), Error> {
    let mut uncontrolled_elements = vec![];
    while let Some(request) =
        stream.try_next().await.context("Error handling element manager request stream")?
    {
        match request {
            ElementManagerRequest::ProposeElement { spec, element_controller, responder } => {
                let mut child_name: String =
                    thread_rng().sample_iter(&Alphanumeric).take(16).collect();
                child_name.make_ascii_lowercase();

                let mut result = match element_manager
                    .launch_element(spec, &child_name, ELEMENT_COLLECTION_NAME)
                    .await
                {
                    Ok(element) => {
                        match element_controller {
                            Some(element_controller) => match element_controller.into_stream() {
                                Ok(stream) => {
                                    handle_element_controller_request_stream(stream, element);
                                    Ok(())
                                }
                                Err(_) => Err(ProposeElementError::Rejected),
                            },
                            // If the element proposer did not provide a controller, add the
                            // element to a vector to keep it alive:
                            None => {
                                uncontrolled_elements.push(element);
                                Ok(())
                            }
                        }
                    }
                    // Most of the errors which could be encountered when adding an element are
                    // not the result of an error by the FIDL client. This lists all the cases
                    // explicitly, but it's up to each session to decide how to map the errors.
                    Err(ElementManagerError::UrlMissing { .. }) => {
                        Err(ProposeElementError::NotFound)
                    }
                    Err(ElementManagerError::NotCreated { .. }) => {
                        Err(ProposeElementError::Rejected)
                    }
                    Err(ElementManagerError::NotBound { .. }) => Err(ProposeElementError::Rejected),
                    Err(ElementManagerError::NotLaunched { .. }) => {
                        Err(ProposeElementError::Rejected)
                    }
                };

                let _ = responder.send(&mut result);
            }
        }
    }
    Ok(())
}

/// Handles the ElementController requests.
///
/// # Parameters
/// - `stream`: the input channel which receives [`ElementController`] requests.
/// - `element`: the [`Element`] that is being controlled.
///
/// # Returns
/// () when there are no more valid requests.
fn handle_element_controller_request_stream(
    mut stream: ElementControllerRequestStream,
    mut element: Element,
) {
    fasync::spawn(async move {
        while let Ok(Some(request)) = stream.try_next().await {
            match request {
                ElementControllerRequest::SetAnnotations { annotations, responder } => {
                    let _ = responder.send(
                        &mut element
                            .set_annotations(annotations)
                            .map_err(|_: anyhow::Error| AnnotationError::Rejected),
                    );
                }
                ElementControllerRequest::GetAnnotations { responder } => {
                    let mut annotations = element.get_annotations();
                    if annotations.is_err() {
                        // GetAnnotations does not return errors, so just return empty annotations.
                        annotations =
                            Ok(Annotations { custom_annotations: None, ..Annotations::new_empty() })
                    }
                    let _ = responder.send(annotations.unwrap());
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn dummy_test() {
        println!("Don't panic!(), you've got this!");
    }
}
