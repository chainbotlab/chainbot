use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use std::sync::Once;

static LOOPBACK_OVERRIDE: Once = Once::new();

#[derive(Clone)]
struct FixtureState {
    redirect_target: String,
}

pub struct HttpFixture {
    pub base_url: String,
    shutdown: Option<oneshot::Sender<()>>,
}

impl HttpFixture {
    pub async fn start() -> Self {
        LOOPBACK_OVERRIDE.call_once(|| {
            unsafe {
                std::env::set_var("CHAINBOT_HTTP_NODE_ALLOW_LOOPBACK_FOR_TESTS", "1");
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fixture listener should bind");
        let address = listener.local_addr().expect("fixture listener should expose address");
        let base_url = format!("http://{}:{}", address.ip(), address.port());
        let state = FixtureState {
            redirect_target: format!("{base_url}/echo"),
        };
        let app = Router::new()
            .route("/echo", get(get_echo).post(post_echo))
            .route("/no-content", get(no_content))
            .route("/redirect", get(redirect_once))
            .route("/binary", get(binary_payload))
            .route("/auth", get(auth_required))
            .with_state(state);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });
            let _ = server.await;
        });
        Self {
            base_url,
            shutdown: Some(shutdown_tx),
        }
    }
}

impl Drop for HttpFixture {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

async fn get_echo() -> impl IntoResponse {
    Json(json!({"message": "ok"}))
}

async fn post_echo(headers: HeaderMap, body: String) -> impl IntoResponse {
    Json(json!({
        "body": body,
        "authorization": headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
    }))
}

async fn no_content() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

async fn redirect_once(State(state): State<FixtureState>) -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::LOCATION,
        HeaderValue::from_str(&state.redirect_target).expect("redirect target should be valid"),
    );
    (StatusCode::FOUND, headers).into_response()
}

async fn binary_payload() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    (StatusCode::OK, headers, vec![0_u8, 159_u8, 146_u8, 150_u8]).into_response()
}

async fn auth_required(headers: HeaderMap) -> impl IntoResponse {
    let authorized = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        == Some("Bearer fixture-token");
    if authorized {
        Json(json!({"authorized": true})).into_response()
    } else {
        (StatusCode::UNAUTHORIZED, Json(json!({"authorized": false}))).into_response()
    }
}

pub fn private_ip_url() -> String {
    String::from("http://10.0.0.1/private")
}

pub fn metadata_ip_url() -> String {
    String::from("http://169.254.169.254/latest/meta-data")
}

pub fn origin(url: &str) -> String {
    let parsed = reqwest::Url::parse(url).expect("fixture url should parse");
    let host = parsed.host_str().expect("fixture url should include host");
    let port = parsed.port_or_known_default().expect("fixture url should expose port");
    format!("{}://{}:{}", parsed.scheme(), host, port)
}

pub fn path_url(base_url: &str, path: &str) -> String {
    format!("{base_url}{path}")
}
