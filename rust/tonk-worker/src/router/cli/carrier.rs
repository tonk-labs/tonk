//! Worker-owned carrier requests. A private reply port binds each dial to
//! a controlled top-level client; unsolicited carrier messages are rejected.

use serde::Serialize;
use std::{cell::RefCell, rc::Rc, sync::Arc};
use wasm_bindgen::{JsCast, closure::Closure};
use wasm_bindgen_futures::JsFuture;

use super::{Reach, deadline, demand::Context};
use crate::router::ClientId;

struct Reply {
    port: web_sys::MessagePort,
    committed: bool,
    _callback: Closure<dyn FnMut(web_sys::MessageEvent)>,
}

impl Drop for Reply {
    fn drop(&mut self) {
        if !self.committed {
            if let Ok(cancel) = serde_json::json!({"v": 1, "type": "cancel"})
                .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
            {
                let _ = self.port.post_message(&cancel);
            }
        }
        self.port.set_onmessage(None);
        self.port.close();
    }
}

/// Prefer a visible, initiating, focused page. Skip stale profile clients and
/// try at most three candidates, eight seconds each. The caller also bounds
/// the complete queue + setup operation. Authentication remains iroh's job.
pub(super) async fn connect(
    context: &Arc<Context>,
    reach: &Arc<Reach>,
    origin: Option<&ClientId>,
    peer: &dialog_iroh_remote::site::IrohAddress,
    address: &tonk_rtc::Address,
) -> Result<tonk_rtc::transport::Inbound, String> {
    let global: web_sys::ServiceWorkerGlobalScope = js_sys::global()
        .dyn_into()
        .map_err(|_| "not running in a service worker")?;
    let clients = JsFuture::from(global.clients().match_all())
        .await
        .map_err(|error| format!("could not find a carrier page: {error:?}"))?;
    let mut clients: Vec<web_sys::WindowClient> = js_sys::Array::from(&clients)
        .iter()
        .filter_map(|value| value.dyn_into::<web_sys::WindowClient>().ok())
        .filter(|client| client.frame_type() == web_sys::FrameType::TopLevel)
        .collect();
    clients.sort_by_key(|client| {
        (
            client.visibility_state() != web_sys::VisibilityState::Visible,
            origin.is_none_or(|origin| origin.0 != client.id()),
            !client.focused(),
        )
    });
    let mut attempts = 0;
    let mut detail =
        "open or reload a Tonk page to carry this profile's local connection".to_owned();
    for client in clients {
        if !context.current() {
            return Err("profile changed while selecting a carrier".into());
        }
        let client_id = ClientId(client.id());
        if !context.client_current(&client_id).await {
            continue;
        }
        attempts += 1;
        match request(context, reach, &client, peer, address).await {
            Ok(lease) => return Ok(lease),
            Err(error) => detail = error,
        }
        if attempts >= 3 {
            break;
        }
    }
    Err(detail)
}

async fn request(
    context: &Arc<Context>,
    reach: &Arc<Reach>,
    client: &web_sys::WindowClient,
    peer: &dialog_iroh_remote::site::IrohAddress,
    address: &tonk_rtc::Address,
) -> Result<tonk_rtc::transport::Inbound, String> {
    let Some(iroh::TransportAddr::Custom(route)) = peer.addr().addrs.iter().next() else {
        return Err("missing WebRTC route".into());
    };
    let channel = web_sys::MessageChannel::new()
        .map_err(|e| format!("could not create carrier request: {e:?}"))?;
    let port = channel.port1();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = Rc::new(RefCell::new(Some(sender)));
    let callback =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let Some(sender) = sender.borrow_mut().take() else {
                return;
            };
            let envelope = serde_wasm_bindgen::from_value::<serde_json::Value>(event.data());
            let result = match envelope {
                Ok(value) if value["v"] == 1 && value["type"] == "carrier" => event
                    .ports()
                    .get(0)
                    .dyn_into::<web_sys::MessagePort>()
                    .map_err(|_| "the page returned no carrier port".to_owned()),
                Ok(value) => Err(value["detail"]
                    .as_str()
                    .unwrap_or("invalid carrier response")
                    .to_owned()),
                Err(error) => Err(format!("invalid carrier response: {error}")),
            };
            if let Err(Ok(port)) = sender.send(result) {
                port.close();
            }
        });
    port.set_onmessage(Some(callback.as_ref().unchecked_ref()));
    port.start();
    let mut reply = Reply {
        port,
        committed: false,
        _callback: callback,
    };
    let request = serde_json::json!({ "v": 1, "type": "tonk-rtc-dial", "peer": peer.did().to_string(), "address": address })
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible()).map_err(|e| e.to_string())?;
    client
        .post_message_with_transfer(&request, &js_sys::Array::of1(&channel.port2()))
        .map_err(|e| format!("could not request a carrier: {e:?}"))?;
    let port = deadline(receiver, 8)
        .await?
        .map_err(|_| "carrier page stopped answering")??;
    if !context.client_current(&ClientId(client.id())).await {
        port.close();
        return Err("carrier rejected after a profile change".into());
    }
    let lease = tonk_rtc::transport::relay::attach(&reach.transport, route.clone(), port);
    let ready = serde_json::json!({ "v": 1, "type": "ready" })
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| e.to_string())?;
    if let Err(error) = reply.port.post_message(&ready) {
        lease.detach();
        return Err(format!("could not acknowledge the carrier: {error:?}"));
    }
    reply.committed = true;
    let context = context.clone();
    let watching = lease.clone();
    wasm_bindgen_futures::spawn_local(async move {
        while watching.is_current() {
            if !context.current() {
                watching.detach();
                break;
            }
            let _ = crate::r#async::sleep(web_time::Duration::from_secs(3)).await;
        }
    });
    Ok(lease)
}
