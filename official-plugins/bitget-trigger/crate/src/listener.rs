use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use tokio::time::{interval, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

use crate::contract::{
    build_bitget_subscribe_request, build_event_frame, event_message, ready_message,
    TriggerStartCommand,
};

type HmacSha256 = Hmac<Sha256>;

const BLOCKED_HOSTS: &[&str] = &["metadata.google.internal", "metadata.azure.internal"];

pub async fn run_listener(command: TriggerStartCommand) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    run_listener_with_writer(command, &mut stdout).await
}

pub async fn run_listener_with_writer(
    command: TriggerStartCommand,
    stdout: &mut impl Write,
) -> Result<(), String> {
    let source = command.source.as_str();
    if source != "bitget_market_stream" && source != "bitget_user_stream" {
        return Err(format!("unsupported Bitget trigger source {source}"));
    }

    let endpoint = command
        .params
        .get("endpoint")
        .and_then(Value::as_str)
        .unwrap_or(match source {
            "bitget_market_stream" => "wss://ws.bitget.com/v2/ws/public",
            _ => "wss://ws.bitget.com/v2/ws/private",
        });

    if endpoint.starts_with("mock://") {
        emit_mock_event(&command, stdout)?;
        return Ok(());
    }

    let requires_binding = source == "bitget_user_stream";
    let parsed_endpoint = validate_destination(
        endpoint,
        command
            .activation
            .as_ref()
            .map(|activation| activation.allowed_origins.as_slice())
            .unwrap_or(&[]),
        requires_binding,
    )?;

    let (mut socket, _) = connect_async(parsed_endpoint.as_str())
        .await
        .map_err(|error| format!("websocket connect failed: {error}"))?;

    if source == "bitget_user_stream" {
        let login_message = build_login_request(&command)?;
        socket
            .send(Message::Text(login_message.to_string().into()))
            .await
            .map_err(|error| format!("failed to send login request: {error}"))?;
        await_login_ack(&mut socket).await?;
    }

    let subscribe = build_bitget_subscribe_request(&command)?;
    socket
        .send(Message::Text(subscribe.to_string().into()))
        .await
        .map_err(|error| format!("failed to send subscribe request: {error}"))?;

    let heartbeat_ms = if command.heartbeat_interval_ms > 0 {
        command.heartbeat_interval_ms as u64
    } else {
        30_000
    };
    let mut ticker = interval(Duration::from_millis(heartbeat_ms));
    let mut ready_emitted = false;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                socket.send(Message::Text(String::from("ping").into()))
                    .await
                    .map_err(|error| format!("failed to send ping: {error}"))?;
            }
            message = socket.next() => {
                let Some(message) = message else {
                    return Ok(());
                };
                let message = message.map_err(|error| format!("websocket receive failed: {error}"))?;
                match message {
                    Message::Text(text) => {
                        if text == "pong" {
                            continue;
                        }
                        let payload: Value = serde_json::from_str(&text)
                            .map_err(|error| format!("bitget websocket returned invalid json: {error}"))?;
                        if is_error_event(&payload) {
                            return Err(stream_error(&payload));
                        }
                        if is_subscribe_ack(&payload) {
                            if !ready_emitted {
                                write_ready(stdout)?;
                                ready_emitted = true;
                            }
                            continue;
                        }
                        if payload.get("event").and_then(Value::as_str) == Some("login") {
                            continue;
                        }
                        if !is_data_event(&payload) {
                            continue;
                        }
                        if !ready_emitted {
                            write_ready(stdout)?;
                            ready_emitted = true;
                        }
                        emit_live_event(&command, stdout, &payload)?;
                    }
                    Message::Ping(payload) => {
                        socket.send(Message::Pong(payload)).await.map_err(|error| format!("failed to reply pong: {error}"))?;
                    }
                    Message::Pong(_) => {}
                    Message::Close(_) => return Ok(()),
                    _ => {}
                }
            }
        }
    }
}

fn emit_mock_event(command: &TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    write_ready(stdout)?;
    let source = command.source.as_str();
    let payload = json!({
        "exchange": "bitget",
        "product_line": command.params.get("product_line").and_then(Value::as_str).unwrap_or("spot"),
        "listener_kind": listener_kind(source),
        "stream": command.params.get("stream").and_then(Value::as_str).unwrap_or(source),
        "event_id": format!("{}:{}", source, command.trigger_id),
        "payload": {"mock": true, "source": source},
    });
    let event = event_message(build_event_frame(
        format!("{}:mock", command.trigger_id),
        format!("{}:event", source),
        current_time_ms()?,
        payload,
    ));
    writeln!(stdout, "{event}").map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())
}

async fn await_login_ack<S>(socket: &mut tokio_tungstenite::WebSocketStream<S>) -> Result<(), String>
where
    tokio_tungstenite::WebSocketStream<S>: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    while let Some(message) = socket.next().await {
        let message = message.map_err(|error| format!("websocket receive failed: {error}"))?;
        match message {
            Message::Text(text) => {
                if text == "pong" {
                    continue;
                }
                let payload: Value = serde_json::from_str(&text)
                    .map_err(|error| format!("bitget websocket returned invalid json: {error}"))?;
                if is_error_event(&payload) {
                    return Err(stream_error(&payload));
                }
                if payload.get("event").and_then(Value::as_str) == Some("login")
                    && payload.get("code").and_then(Value::as_str) == Some("0")
                {
                    return Ok(());
                }
            }
            Message::Ping(payload) => {
                socket.send(Message::Pong(payload)).await.map_err(|error| format!("failed to reply pong: {error}"))?;
            }
            Message::Close(_) => return Err(String::from("websocket closed before login ack")),
            _ => {}
        }
    }
    Err(String::from("websocket ended before login ack"))
}

fn emit_live_event(
    command: &TriggerStartCommand,
    stdout: &mut impl Write,
    payload: &Value,
) -> Result<(), String> {
    let stream = event_stream_name(command, payload);
    let event_key = event_key(command, &stream, payload);
    let occurred_at_ms = event_time_ms(payload).unwrap_or(current_time_ms()?);
    let normalized = normalize_payload(command, &stream, payload, &event_key);
    let checkpoint = format!("{}:{}:{}", command.source, stream, event_key);
    let frame = build_event_frame(checkpoint, event_key, occurred_at_ms, normalized);
    writeln!(stdout, "{}", event_message(frame)).map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())
}

fn write_ready(stdout: &mut impl Write) -> Result<(), String> {
    writeln!(stdout, "{}", ready_message()).map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())
}

fn build_login_request(command: &TriggerStartCommand) -> Result<Value, String> {
    let activation = command
        .activation
        .as_ref()
        .ok_or_else(|| String::from("activation is required for bitget_user_stream"))?;
    let api_key = activation
        .secrets
        .get("api_key")
        .ok_or_else(|| String::from("activation secret api_key is required"))?;
    let api_secret = activation
        .secrets
        .get("api_secret")
        .ok_or_else(|| String::from("activation secret api_secret is required"))?;
    let passphrase = activation
        .secrets
        .get("passphrase")
        .ok_or_else(|| String::from("activation secret passphrase is required"))?;
    let timestamp = current_time_seconds()?.to_string();
    let signature = sign_ws_login(&timestamp, api_secret)?;
    Ok(json!({
        "op": "login",
        "args": [{
            "apiKey": api_key,
            "passphrase": passphrase,
            "timestamp": timestamp,
            "sign": signature,
        }]
    }))
}

fn sign_ws_login(timestamp: &str, secret: &str) -> Result<String, String> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|error| error.to_string())?;
    mac.update(format!("{timestamp}GET/user/verify").as_bytes());
    Ok(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}

fn normalize_payload(
    command: &TriggerStartCommand,
    stream: &str,
    payload: &Value,
    event_key: &str,
) -> Value {
    json!({
        "exchange": "bitget",
        "product_line": product_line(command, payload),
        "listener_kind": listener_kind(&command.source),
        "stream": stream,
        "event_id": event_key,
        "payload": payload.get("data").cloned().unwrap_or_else(|| payload.clone()),
    })
}

fn product_line(command: &TriggerStartCommand, payload: &Value) -> String {
    if let Some(value) = command.params.get("product_line").and_then(Value::as_str) {
        return value.to_string();
    }
    let inst_type = payload
        .get("arg")
        .and_then(|arg| arg.get("instType"))
        .and_then(Value::as_str)
        .unwrap_or("SPOT");
    if inst_type.eq_ignore_ascii_case("SPOT") {
        String::from("spot")
    } else {
        String::from("futures")
    }
}

fn listener_kind(source: &str) -> &'static str {
    if source == "bitget_market_stream" {
        "market_stream"
    } else {
        "user_stream"
    }
}

fn event_stream_name(command: &TriggerStartCommand, payload: &Value) -> String {
    payload
        .get("arg")
        .and_then(|arg| arg.get("channel"))
        .and_then(Value::as_str)
        .or_else(|| command.params.get("stream").and_then(Value::as_str))
        .or_else(|| command.params.get("channel").and_then(Value::as_str))
        .unwrap_or(listener_kind(&command.source))
        .to_string()
}

fn event_key(command: &TriggerStartCommand, stream: &str, payload: &Value) -> String {
    let discriminator = payload
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| {
            item.get("orderId")
                .or_else(|| item.get("clientOid"))
                .or_else(|| item.get("ts"))
                .or_else(|| item.get("uTime"))
                .or_else(|| item.get("cTime"))
                .cloned()
        })
        .or_else(|| payload.get("ts").cloned())
        .unwrap_or_else(|| json!("event"));
    let suffix = match discriminator {
        Value::String(value) => value,
        Value::Number(value) => value.to_string(),
        other => other.to_string(),
    };
    format!("{}:{}:{}", listener_kind(&command.source), stream, suffix)
}

fn event_time_ms(payload: &Value) -> Option<i64> {
    payload
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("ts").or_else(|| item.get("uTime")).or_else(|| item.get("cTime")))
        .and_then(value_to_i64)
        .or_else(|| payload.get("ts").and_then(value_to_i64))
}

fn value_to_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.parse::<i64>().ok(),
        _ => None,
    }
}

fn is_subscribe_ack(payload: &Value) -> bool {
    payload.get("event").and_then(Value::as_str) == Some("subscribe")
}

fn is_error_event(payload: &Value) -> bool {
    payload.get("event").and_then(Value::as_str) == Some("error")
}

fn is_data_event(payload: &Value) -> bool {
    payload.get("data").and_then(Value::as_array).is_some()
}

fn stream_error(payload: &Value) -> String {
    let code = payload.get("code").and_then(Value::as_str).unwrap_or("unknown");
    let message = payload.get("msg").and_then(Value::as_str).unwrap_or("unknown error");
    format!("bitget stream error [{code}] {message}")
}

fn validate_destination(
    url: &str,
    allowed_origins: &[String],
    requires_binding: bool,
) -> Result<Url, String> {
    let parsed = Url::parse(url).map_err(|error| format!("invalid destination url: {error}"))?;
    validate_destination_policy(&parsed)?;
    validate_allowed_origins(&parsed, allowed_origins, requires_binding)?;
    Ok(parsed)
}

fn validate_allowed_origins(
    url: &Url,
    allowed_origins: &[String],
    requires_binding: bool,
) -> Result<(), String> {
    if !requires_binding {
        return Ok(());
    }
    if allowed_origins.is_empty() {
        return Err(String::from(
            "activation secrets require at least one allowed origin",
        ));
    }
    let origin = normalize_origin(url)?;
    let permitted = allowed_origins.iter().any(|item| {
        Url::parse(item)
            .ok()
            .and_then(|value| normalize_origin(&value).ok())
            .as_deref()
            == Some(origin.as_str())
    });
    if permitted {
        Ok(())
    } else {
        Err(format!(
            "request origin {} is not allowlisted for activation secrets",
            origin
        ))
    }
}

fn normalize_origin(url: &Url) -> Result<String, String> {
    let scheme = match url.scheme() {
        "ws" | "http" => "http",
        "wss" | "https" => "https",
        other => return Err(format!("unsupported destination scheme {other}")),
    };
    let host = url
        .host_str()
        .ok_or_else(|| String::from("destination url must include a host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| String::from("destination url must use a known port"))?;
    Ok(format!("{scheme}://{host}:{port}"))
}

fn validate_destination_policy(url: &Url) -> Result<Vec<SocketAddr>, String> {
    if !matches!(url.scheme(), "https" | "wss")
        && !(matches!(url.scheme(), "http" | "ws")
            && allow_loopback_for_tests()
            && is_loopback_host(url.host_str()))
    {
        return Err(String::from("destination url must use https or wss"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| String::from("destination url must include a host"))?;
    if BLOCKED_HOSTS
        .iter()
        .any(|blocked| host.eq_ignore_ascii_case(blocked))
    {
        return Err(format!("destination host `{host}` is blocked"));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| String::from("destination url must use a known port"))?;
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("failed to resolve destination host: {error}"))?
        .collect::<Vec<_>>();
    for address in &addresses {
        validate_ip(address.ip())?;
    }
    Ok(addresses)
}

fn is_loopback_host(host: Option<&str>) -> bool {
    matches!(host, Some("localhost") | Some("127.0.0.1") | Some("::1"))
}

fn validate_ip(ip: IpAddr) -> Result<(), String> {
    match ip {
        IpAddr::V4(ip) => {
            if ip.is_loopback() && allow_loopback_for_tests() {
                return Ok(());
            }
            if ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
            {
                return Err(format!("destination address `{ip}` is blocked"));
            }
        }
        IpAddr::V6(ip) => {
            if ip.is_loopback() && allow_loopback_for_tests() {
                return Ok(());
            }
            if ip.is_loopback()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
            {
                return Err(format!("destination address `{ip}` is blocked"));
            }
        }
    }
    Ok(())
}

fn allow_loopback_for_tests() -> bool {
    cfg!(debug_assertions)
        && matches!(
            std::env::var("CHAINBOT_HTTP_NODE_ALLOW_LOOPBACK_FOR_TESTS").as_deref(),
            Ok("1")
        )
        && matches!(
            std::env::var("CHAINBOT_INTERNAL_ALLOW_TEST_DESTINATIONS").as_deref(),
            Ok("1")
        )
}

fn current_time_ms() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(duration.as_millis()).map_err(|error| error.to_string())
}

fn current_time_seconds() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(duration.as_secs()).map_err(|error| error.to_string())
}
