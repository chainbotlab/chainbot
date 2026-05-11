use std::net::{IpAddr, ToSocketAddrs};

use url::Url;

const BLOCKED_HOSTS: &[&str] = &["metadata.google.internal", "metadata.azure.internal"];

pub fn normalize_base_url(input: &str) -> String {
    input.trim_end_matches('/').to_owned()
}

pub fn validate_destination_policy(
    url: &str,
    allowed_origins: &[String],
    requires_binding: bool,
) -> Result<(), String> {
    let parsed = Url::parse(url).map_err(|error| format!("invalid destination url: {error}"))?;
    validate_scheme(&parsed)?;
    validate_allowed_origins(&parsed, allowed_origins, requires_binding)?;
    validate_ip_resolution(&parsed)?;
    Ok(())
}

fn validate_scheme(url: &Url) -> Result<(), String> {
    if matches!(url.scheme(), "https" | "wss") {
        return Ok(());
    }
    if matches!(url.scheme(), "http" | "ws") && allow_loopback_for_tests() && is_loopback_host(url.host_str()) {
        return Ok(());
    }
    Err(String::from("destination url must use https or wss"))
}

fn is_loopback_host(host: Option<&str>) -> bool {
    matches!(host, Some("localhost") | Some("127.0.0.1") | Some("::1"))
}

fn validate_allowed_origins(url: &Url, allowed_origins: &[String], requires_binding: bool) -> Result<(), String> {
    if !requires_binding {
        return Ok(());
    }
    if allowed_origins.is_empty() {
        return Err(String::from("activation secrets require at least one allowed origin"));
    }
    let request_origin = normalize_origin(url)?;
    let permitted = allowed_origins.iter().any(|origin| {
        Url::parse(origin)
            .ok()
            .and_then(|parsed| normalize_origin(&parsed).ok())
            .as_deref()
            == Some(request_origin.as_str())
    });
    if permitted {
        Ok(())
    } else {
        Err(format!(
            "request origin {} is not allowlisted for activation secrets",
            request_origin
        ))
    }
}

fn normalize_origin(url: &Url) -> Result<String, String> {
    let scheme = match url.scheme() {
        "http" | "ws" => "http",
        "https" | "wss" => "https",
        other => return Err(format!("unsupported origin scheme {other}")),
    };
    let host = url.host_str().ok_or_else(|| String::from("destination url must include a host"))?;
    let port = url.port_or_known_default().ok_or_else(|| String::from("destination url must use a known port"))?;
    Ok(format!("{scheme}://{host}:{port}"))
}

fn validate_ip_resolution(url: &Url) -> Result<(), String> {
    let host = url.host_str().ok_or_else(|| String::from("destination url must include a host"))?;
    if BLOCKED_HOSTS.iter().any(|blocked| host.eq_ignore_ascii_case(blocked)) {
        return Err(format!("destination host `{host}` is blocked"));
    }
    let port = url.port_or_known_default().ok_or_else(|| String::from("destination url must use a known port"))?;
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("failed to resolve destination host: {error}"))?
        .collect::<Vec<_>>();
    for address in &addresses {
        validate_ip(address.ip())?;
    }
    Ok(())
}

fn validate_ip(ip: IpAddr) -> Result<(), String> {
    match ip {
        IpAddr::V4(ip) => {
            if ip.is_loopback() && allow_loopback_for_tests() {
                return Ok(());
            }
            let octets = ip.octets();
            let is_shared_range = octets[0] == 100 && (64..=127).contains(&octets[1]);
            if ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
                || is_shared_range
                || ip.octets() == [169, 254, 169, 254]
            {
                return Err(format!("destination address `{ip}` is blocked"));
            }
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return validate_ip(IpAddr::V4(mapped));
            }
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
        && matches!(std::env::var("CHAINBOT_HTTP_NODE_ALLOW_LOOPBACK_FOR_TESTS").as_deref(), Ok("1"))
        && matches!(std::env::var("CHAINBOT_INTERNAL_ALLOW_TEST_DESTINATIONS").as_deref(), Ok("1"))
}
