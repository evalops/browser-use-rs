use crate::{BrowserError, DevToolsEndpoint};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

const CDP_RECONNECT_MAX_ATTEMPTS: u32 = 3;
const CDP_RECONNECT_DELAYS_MS: [u64; 3] = [1_000, 2_000, 4_000];
const CDP_CONNECT_TIMEOUT_MS: u64 = 15_000;
type CdpSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct CdpConnection {
    pub(crate) request_tx: mpsc::Sender<CdpRequest>,
    pub(crate) event_tx: broadcast::Sender<CdpEvent>,
    pub(crate) next_id: AtomicU64,
    pub(crate) intentional_stop: Arc<AtomicBool>,
    pub(crate) connection_generation: Arc<AtomicU64>,
    pub(crate) session_generations: Arc<Mutex<HashMap<String, u64>>>,
}

pub(crate) struct CdpRequest {
    id: u64,
    method: String,
    payload: Value,
    response_tx: oneshot::Sender<Result<Value, BrowserError>>,
}

struct CdpSocketConfig {
    cdp_url: String,
    websocket_url: String,
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CdpEvent {
    pub(crate) method: String,
    pub(crate) params: Value,
    pub(crate) session_id: Option<String>,
}

impl CdpConnection {
    pub async fn connect(endpoint: &DevToolsEndpoint) -> Result<Arc<Self>, BrowserError> {
        Self::connect_with_headers(endpoint, None).await
    }

    pub(crate) async fn connect_with_headers(
        endpoint: &DevToolsEndpoint,
        headers: Option<&BTreeMap<String, String>>,
    ) -> Result<Arc<Self>, BrowserError> {
        let headers = headers.cloned().unwrap_or_default();
        let socket = connect_cdp_socket(&endpoint.websocket_url, &headers).await?;
        let (request_tx, request_rx) = mpsc::channel(64);
        let (event_tx, _) = broadcast::channel(256);
        let intentional_stop = Arc::new(AtomicBool::new(false));
        let connection_generation = Arc::new(AtomicU64::new(0));
        let session_generations = Arc::new(Mutex::new(HashMap::new()));
        let socket_config = CdpSocketConfig {
            cdp_url: endpoint.http_url.clone(),
            websocket_url: endpoint.websocket_url.clone(),
            headers,
        };
        tokio::spawn(cdp_connection_actor(
            socket_config,
            socket,
            request_rx,
            event_tx.clone(),
            intentional_stop.clone(),
            connection_generation.clone(),
        ));

        Ok(Arc::new(Self {
            request_tx,
            event_tx,
            next_id: AtomicU64::new(1),
            intentional_stop,
            connection_generation,
            session_generations,
        }))
    }

    pub(crate) fn subscribe_events(&self) -> broadcast::Receiver<CdpEvent> {
        self.event_tx.subscribe()
    }

    pub(crate) fn mark_intentional_stop(&self) {
        self.intentional_stop.store(true, Ordering::Relaxed);
    }

    fn current_generation(&self) -> u64 {
        self.connection_generation.load(Ordering::Relaxed)
    }

    pub(crate) async fn register_attached_session(&self, session_id: &str) {
        self.session_generations
            .lock()
            .await
            .insert(session_id.to_owned(), self.current_generation());
    }

    pub(crate) async fn ensure_session_generation_current(
        &self,
        session_id: Option<&str>,
    ) -> Result<(), BrowserError> {
        let Some(session_id) = session_id else {
            return Ok(());
        };
        if self.is_registered_session_stale(session_id).await {
            return Err(BrowserError::Transport(format!(
                "CDP session {session_id} is stale after reconnect; reattach target before sending session-scoped commands"
            )));
        }
        Ok(())
    }

    pub(crate) async fn is_registered_session_stale(&self, session_id: &str) -> bool {
        let Some(session_generation) = self
            .session_generations
            .lock()
            .await
            .get(session_id)
            .copied()
        else {
            return false;
        };
        session_generation != self.current_generation()
    }

    pub async fn command(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, BrowserError> {
        self.ensure_session_generation_current(session_id).await?;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut request = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        if let Some(session_id) = session_id {
            request["sessionId"] = Value::String(session_id.to_owned());
        }

        let (response_tx, response_rx) = oneshot::channel();
        self.request_tx
            .send(CdpRequest {
                id,
                method: method.to_owned(),
                payload: request,
                response_tx,
            })
            .await
            .map_err(|error| BrowserError::Transport(error.to_string()))?;

        response_rx
            .await
            .map_err(|_| BrowserError::Transport("CDP command actor stopped".to_owned()))?
    }
}

async fn connect_cdp_socket(
    websocket_url: &str,
    headers: &BTreeMap<String, String>,
) -> Result<CdpSocket, BrowserError> {
    let request = cdp_websocket_request(websocket_url, headers)?;
    let connect_result = tokio::time::timeout(
        Duration::from_millis(CDP_CONNECT_TIMEOUT_MS),
        connect_async(request),
    )
    .await
    .map_err(|_| {
        BrowserError::Transport(format!(
            "CDP websocket connect to {websocket_url} timed out after {CDP_CONNECT_TIMEOUT_MS}ms"
        ))
    })?;
    connect_result
        .map(|(socket, _)| socket)
        .map_err(|error| BrowserError::Transport(error.to_string()))
}

pub(crate) fn cdp_websocket_request(
    websocket_url: &str,
    headers: &BTreeMap<String, String>,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, BrowserError> {
    let mut request = websocket_url
        .into_client_request()
        .map_err(|error| BrowserError::Transport(error.to_string()))?;
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
            BrowserError::Transport(format!(
                "invalid CDP websocket header name {name:?}: {error}"
            ))
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|error| {
            BrowserError::Transport(format!(
                "invalid CDP websocket header value for {name:?}: {error}"
            ))
        })?;
        request.headers_mut().insert(header_name, header_value);
    }
    Ok(request)
}

async fn cdp_connection_actor(
    socket_config: CdpSocketConfig,
    mut socket: CdpSocket,
    mut request_rx: mpsc::Receiver<CdpRequest>,
    event_tx: broadcast::Sender<CdpEvent>,
    intentional_stop: Arc<AtomicBool>,
    connection_generation: Arc<AtomicU64>,
) {
    let mut pending: HashMap<u64, (String, oneshot::Sender<Result<Value, BrowserError>>)> =
        HashMap::new();

    loop {
        let websocket_closed_event = loop {
            tokio::select! {
                Some(request) = request_rx.recv() => {
                    let text = request.payload.to_string();
                    match socket.send(Message::Text(text.into())).await {
                        Ok(()) => {
                            pending.insert(request.id, (request.method, request.response_tx));
                        }
                        Err(error) => {
                            let _ = request.response_tx.send(Err(BrowserError::Transport(error.to_string())));
                        }
                    }
                }
                message = socket.next() => {
                    let Some(message) = message else {
                        break cdp_websocket_closed_event("websocket_stream_ended", None);
                    };
                    let payload = match message {
                        Ok(Message::Text(text)) => match serde_json::from_str::<Value>(&text) {
                            Ok(payload) => payload,
                            Err(error) => {
                                let _ = event_tx.send(CdpEvent {
                                    method: "browser-use-rs.invalid-json".to_owned(),
                                    params: json!({ "error": error.to_string() }),
                                    session_id: None,
                                });
                                continue;
                            }
                        },
                        Ok(_) => continue,
                        Err(error) => {
                            let error = error.to_string();
                            let transport_error = BrowserError::Transport(error.clone());
                            for (_, (_, response_tx)) in pending.drain() {
                                let _ = response_tx.send(Err(transport_error.clone()));
                            }
                            break cdp_websocket_closed_event("websocket_error", Some(error));
                        }
                    };

                    if let Some(id) = payload.get("id").and_then(Value::as_u64) {
                        let Some((method, response_tx)) = pending.remove(&id) else {
                            continue;
                        };
                        let result = if let Some(error) = payload.get("error") {
                            let message = error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown CDP error")
                                .to_owned();
                            Err(BrowserError::CommandFailed { method, message })
                        } else {
                            payload
                                .get("result")
                                .cloned()
                                .ok_or_else(|| BrowserError::MissingResponseData(format!("{method} result")))
                        };
                        let _ = response_tx.send(result);
                        continue;
                    }

                    if let Some(method) = payload.get("method").and_then(Value::as_str) {
                        let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
                        let session_id = payload
                            .get("sessionId")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        let _ = event_tx.send(CdpEvent {
                            method: method.to_owned(),
                            params,
                            session_id,
                        });
                    }
                }
                else => {
                    break cdp_websocket_closed_event("connection_actor_stopped", None);
                }
            }
        };

        let _ = event_tx.send(websocket_closed_event.clone());

        for (_, (_, response_tx)) in pending.drain() {
            let _ = response_tx.send(Err(BrowserError::Transport(
                "CDP websocket closed while waiting for response".to_owned(),
            )));
        }

        if !should_reconnect_after_websocket_event(
            &websocket_closed_event,
            intentional_stop.load(Ordering::Relaxed),
            request_rx.is_closed(),
        ) {
            break;
        }

        match reconnect_cdp_socket(&socket_config, &event_tx, &connection_generation).await {
            Some(reconnected_socket) => {
                socket = reconnected_socket;
            }
            None => break,
        }
    }
}

async fn reconnect_cdp_socket(
    socket_config: &CdpSocketConfig,
    event_tx: &broadcast::Sender<CdpEvent>,
    connection_generation: &AtomicU64,
) -> Option<CdpSocket> {
    let started_at = Instant::now();
    let mut last_error = None;

    for attempt in 1..=CDP_RECONNECT_MAX_ATTEMPTS {
        let _ = event_tx.send(cdp_websocket_reconnecting_event(
            &socket_config.cdp_url,
            attempt,
            CDP_RECONNECT_MAX_ATTEMPTS,
        ));

        match connect_cdp_socket(&socket_config.websocket_url, &socket_config.headers).await {
            Ok(socket) => {
                let generation = connection_generation.fetch_add(1, Ordering::Relaxed) + 1;
                let _ = event_tx.send(cdp_websocket_reconnected_event(
                    &socket_config.cdp_url,
                    attempt,
                    started_at.elapsed(),
                    generation,
                ));
                return Some(socket);
            }
            Err(error) => {
                last_error = Some(error.to_string());
                if attempt < CDP_RECONNECT_MAX_ATTEMPTS {
                    sleep(cdp_reconnect_delay_for_attempt(attempt)).await;
                }
            }
        }
    }

    let _ = event_tx.send(cdp_websocket_reconnect_failed_event(
        &socket_config.cdp_url,
        CDP_RECONNECT_MAX_ATTEMPTS,
        started_at.elapsed(),
        last_error,
    ));
    None
}

pub(crate) fn should_reconnect_after_websocket_event(
    event: &CdpEvent,
    intentional_stop: bool,
    request_channel_closed: bool,
) -> bool {
    if intentional_stop || request_channel_closed {
        return false;
    }
    event.method == "browser-use-rs.websocket-closed"
        && event
            .params
            .get("reason")
            .and_then(Value::as_str)
            .is_some_and(|reason| matches!(reason, "websocket_stream_ended" | "websocket_error"))
}

pub(crate) fn cdp_reconnect_delay_for_attempt(attempt: u32) -> Duration {
    let index = attempt.saturating_sub(1) as usize;
    Duration::from_millis(
        CDP_RECONNECT_DELAYS_MS
            .get(index)
            .copied()
            .unwrap_or_else(|| *CDP_RECONNECT_DELAYS_MS.last().expect("nonempty delays")),
    )
}

fn cdp_websocket_closed_event(reason: &str, error: Option<String>) -> CdpEvent {
    let mut params = json!({ "reason": reason });
    if let Some(error) = error {
        params["error"] = Value::String(error);
    }
    CdpEvent {
        method: "browser-use-rs.websocket-closed".to_owned(),
        params,
        session_id: None,
    }
}

pub(crate) fn cdp_websocket_reconnecting_event(
    cdp_url: &str,
    attempt: u32,
    max_attempts: u32,
) -> CdpEvent {
    CdpEvent {
        method: "browser-use-rs.websocket-reconnecting".to_owned(),
        params: json!({
            "cdp_url": cdp_url,
            "attempt": attempt,
            "max_attempts": max_attempts,
        }),
        session_id: None,
    }
}

pub(crate) fn cdp_websocket_reconnected_event(
    cdp_url: &str,
    attempt: u32,
    downtime: Duration,
    generation: u64,
) -> CdpEvent {
    CdpEvent {
        method: "browser-use-rs.websocket-reconnected".to_owned(),
        params: json!({
            "cdp_url": cdp_url,
            "attempt": attempt,
            "downtime_seconds": format!("{:.3}", downtime.as_secs_f64()),
            "connection_generation": generation,
        }),
        session_id: None,
    }
}

pub(crate) fn cdp_websocket_reconnect_failed_event(
    cdp_url: &str,
    max_attempts: u32,
    downtime: Duration,
    error: Option<String>,
) -> CdpEvent {
    let mut params = json!({
        "cdp_url": cdp_url,
        "max_attempts": max_attempts,
        "downtime_seconds": format!("{:.3}", downtime.as_secs_f64()),
    });
    if let Some(error) = error {
        params["error"] = Value::String(error);
    }
    CdpEvent {
        method: "browser-use-rs.websocket-reconnect-failed".to_owned(),
        params,
        session_id: None,
    }
}
