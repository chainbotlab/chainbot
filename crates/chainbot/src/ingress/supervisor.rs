//! [INPUT]
//! Desired ingress listener state, runtime storage configuration, Axum router primitives, and supervisor control messages.
//!
//! [OUTPUT]
//! Starts, reconciles, and gracefully shuts down ingress listener workers for webhook and websocket routes.
//!
//! [ROLE]
//! Owns background supervision of ingress listeners during daemon runtime.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::mpsc;
use std::thread;

use axum::extract::connect_info::IntoMakeServiceWithConnectInfo;
use axum::Router;
use tokio::net::TcpListener;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::infrastructure::config::RuntimeStorageConfig;

use super::contract::{DesiredIngressState, IngressListenerSpec, IngressRuntimeError};
use super::{webhook, websocket};

pub struct TriggerIngressSupervisor {
    control_tx: mpsc::Sender<ControlMessage>,
    worker: Option<thread::JoinHandle<()>>,
}

enum ControlMessage {
    Reconcile {
        desired: DesiredIngressState,
        response_tx: mpsc::Sender<Result<(), IngressRuntimeError>>,
    },
    Shutdown {
        response_tx: mpsc::Sender<Result<(), IngressRuntimeError>>,
    },
}

struct ActiveListener {
    stop_tx: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl TriggerIngressSupervisor {
    pub fn start(storage_config: RuntimeStorageConfig) -> Result<Self, IngressRuntimeError> {
        let (control_tx, control_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name(String::from("chainbot-ingress"))
            .spawn(move || run_supervisor_worker(control_rx, storage_config))
            .map_err(|source| IngressRuntimeError::Runtime(format!("spawn ingress worker: {source}")))?;
        Ok(Self {
            control_tx,
            worker: Some(worker),
        })
    }

    pub fn reconcile(&self, desired: DesiredIngressState) -> Result<(), IngressRuntimeError> {
        let (response_tx, response_rx) = mpsc::channel();
        self.control_tx
            .send(ControlMessage::Reconcile {
                desired,
                response_tx,
            })
            .map_err(|_| IngressRuntimeError::ControlChannelClosed)?;
        response_rx
            .recv()
            .map_err(|_| IngressRuntimeError::ControlChannelClosed)?
    }

    pub fn shutdown(mut self) -> Result<(), IngressRuntimeError> {
        let (response_tx, response_rx) = mpsc::channel();
        self.control_tx
            .send(ControlMessage::Shutdown { response_tx })
            .map_err(|_| IngressRuntimeError::ControlChannelClosed)?;
        response_rx
            .recv()
            .map_err(|_| IngressRuntimeError::ControlChannelClosed)??;
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| IngressRuntimeError::Runtime(String::from("join ingress worker")))?;
        }
        Ok(())
    }
}

fn run_supervisor_worker(control_rx: mpsc::Receiver<ControlMessage>, storage_config: RuntimeStorageConfig) {
    let runtime = match Builder::new_multi_thread().enable_all().worker_threads(2).build() {
        Ok(runtime) => runtime,
        Err(_) => return,
    };
    let mut current_state = DesiredIngressState::default();
    let mut current_fingerprint = current_state.fingerprint();
    let mut listeners = BTreeMap::<String, ActiveListener>::new();

    while let Ok(message) = control_rx.recv() {
        match message {
            ControlMessage::Reconcile { desired, response_tx } => {
                if desired.fingerprint() == current_fingerprint && desired == current_state {
                    let _ = response_tx.send(Ok(()));
                    continue;
                }
                let result = runtime.block_on(async {
                    stop_all_listeners(&mut listeners).await;
                    start_listeners(&runtime, &storage_config, &desired, &mut listeners).await
                });
                if result.is_ok() {
                    current_fingerprint = desired.fingerprint();
                    current_state = desired;
                }
                let _ = response_tx.send(result);
            }
            ControlMessage::Shutdown { response_tx } => {
                runtime.block_on(async { stop_all_listeners(&mut listeners).await });
                let _ = response_tx.send(Ok(()));
                break;
            }
        }
    }
}

async fn stop_all_listeners(listeners: &mut BTreeMap<String, ActiveListener>) {
    let active = std::mem::take(listeners);
    for (_, mut listener) in active {
        if let Some(stop_tx) = listener.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        let _ = listener.task.await;
    }
}

async fn start_listeners(
    runtime: &Runtime,
    storage_config: &RuntimeStorageConfig,
    desired: &DesiredIngressState,
    listeners: &mut BTreeMap<String, ActiveListener>,
) -> Result<(), IngressRuntimeError> {
    let mut grouped = BTreeMap::<String, Vec<IngressListenerSpec>>::new();
    for spec in &desired.listeners {
        grouped
            .entry(spec.bind.clone())
            .or_default()
            .push(spec.clone());
    }

    for (bind, specs) in grouped {
        let router = build_router_for_bind(specs, storage_config.clone())?;
        let listener = TcpListener::bind(&bind)
            .await
            .map_err(|source| IngressRuntimeError::ListenerBind {
                bind: bind.clone(),
                source,
            })?;
        let (stop_tx, stop_rx) = oneshot::channel();
        let task = runtime.spawn(run_axum_listener(listener, router, stop_rx));
        listeners.insert(
            bind,
            ActiveListener {
                stop_tx: Some(stop_tx),
                task,
            },
        );
    }
    Ok(())
}

fn build_router_for_bind(
    specs: Vec<IngressListenerSpec>,
    storage_config: RuntimeStorageConfig,
) -> Result<Router, IngressRuntimeError> {
    let mut router = Router::new();
    for spec in specs {
        router = match spec.transport {
            super::contract::IngressTransportKind::Webhook => {
                router.merge(webhook::build_webhook_route(spec, storage_config.clone())?)
            }
            super::contract::IngressTransportKind::WebSocket => {
                router.merge(websocket::build_websocket_route(spec, storage_config.clone())?)
            }
        };
    }
    Ok(router)
}

async fn run_axum_listener(
    listener: TcpListener,
    router: Router,
    stop_rx: oneshot::Receiver<()>,
) {
    let service: IntoMakeServiceWithConnectInfo<Router, SocketAddr> =
        router.into_make_service_with_connect_info::<SocketAddr>();
    let _ = axum::serve(listener, service)
        .with_graceful_shutdown(async {
            let _ = stop_rx.await;
        })
        .await;
}
