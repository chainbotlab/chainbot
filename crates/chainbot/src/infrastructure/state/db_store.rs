//! [INPUT]
//! Root storage configuration and runtime run/trigger state mutations for local SQLite or PostgreSQL backends.
//!
//! [OUTPUT]
//! Persists, reads, and archives authoritative run and trigger history from database tables used by CLI and trigger runtime paths.
//!
//! [ROLE]
//! Defines DB-primary runtime state for ChainBot main execution commands.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use postgres::{Client, NoTls};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::domain::state::{
    IngressInboxRecord, LeaseAcquireResult, RunRecordSummary, RunStatus, ServeLeaseGrant,
    ServeLeaseSnapshot,
    ServeLeaseState, StagedTriggerEventRecord, TriggerCheckpointRecord, TriggerEventRecord,
    TriggerSnapshotRecord, WorkflowRuntimeLogEntry,
};
use crate::infrastructure::config::{
    RuntimeHistoryRetentionPolicy, RuntimeStorageBackend, RuntimeStorageConfig,
};

const SERVE_LEASE_KEY: &str = "serve";

use crate::domain::trigger::{TriggerAcceptanceCommand, TriggerAcceptanceOutcome, TriggerRunRequest};

#[derive(Debug)]
pub enum RuntimeStateError {
    Io {
        path: PathBuf,
        operation: &'static str,
        source: std::io::Error,
    },
    Sqlite {
        path: PathBuf,
        operation: &'static str,
        source: rusqlite::Error,
    },
    Postgres {
        operation: &'static str,
        source: postgres::Error,
    },
    JsonEncode {
        field: &'static str,
        source: serde_json::Error,
    },
    JsonDecode {
        field: &'static str,
        source: serde_json::Error,
    },
    LeaseFenceLost {
        run_id: String,
    },
}

enum RuntimeStorageConnection {
    Sqlite {
        path: PathBuf,
        connection: Connection,
    },
    Postgres {
        database_url: String,
        client: Client,
    },
}

pub struct RuntimeStateStore {
    connection: RuntimeStorageConnection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub struct RuntimeHistoryArchiveStats {
    pub archived_run_summaries: u64,
    pub archived_workflow_logs: u64,
    pub archived_trigger_events: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub struct RuntimeHistoryArchiveCounts {
    pub run_summaries: u64,
    pub workflow_logs: u64,
    pub trigger_events: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDaemonStatus {
    pub state: ServeLeaseState,
    pub owner_id: Option<String>,
    pub pid: Option<i64>,
    pub started_at_ms: Option<i64>,
    pub last_heartbeat_at_ms: Option<i64>,
    pub lease_expires_at_ms: Option<i64>,
    pub last_reload_at_ms: Option<i64>,
    pub stop_requested_at_ms: Option<i64>,
    pub stopped_at_ms: Option<i64>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DaemonSessionRow {
    owner_id: String,
    pid: Option<i64>,
    started_at_ms: i64,
    last_heartbeat_at_ms: i64,
    lease_expires_at_ms: i64,
    last_reload_at_ms: Option<i64>,
    stop_requested_at_ms: Option<i64>,
    stopped_at_ms: Option<i64>,
    last_error_code: Option<String>,
    last_error_message: Option<String>,
}

impl std::fmt::Debug for RuntimeStorageConnection {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite { path, .. } => f
                .debug_struct("RuntimeStorageConnection::Sqlite")
                .field("path", path)
                .finish(),
            Self::Postgres { database_url, .. } => f
                .debug_struct("RuntimeStorageConnection::Postgres")
                .field("database_url", database_url)
                .finish(),
        }
    }
}

impl std::fmt::Debug for RuntimeStateStore {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeStateStore")
            .field("connection", &self.connection)
            .finish()
    }
}

impl RuntimeStateStore {
    pub fn open(config: &RuntimeStorageConfig, now_ms: i64) -> Result<Self, RuntimeStateError> {
        let connection = match &config.backend {
            RuntimeStorageBackend::Local { database_path } => {
                if let Some(parent) = database_path.parent() {
                    fs::create_dir_all(parent).map_err(|source| RuntimeStateError::Io {
                        path: parent.to_path_buf(),
                        operation: "create sqlite parent directory",
                        source,
                    })?;
                }

                let connection = Connection::open(database_path).map_err(|source| {
                    RuntimeStateError::Sqlite {
                        path: database_path.clone(),
                        operation: "open sqlite database",
                        source,
                    }
                })?;
                connection
                    .busy_timeout(Duration::from_millis(1_000))
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: database_path.clone(),
                        operation: "set sqlite busy timeout",
                        source,
                    })?;

                RuntimeStorageConnection::Sqlite {
                    path: database_path.clone(),
                    connection,
                }
            }
            RuntimeStorageBackend::Postgres { database_url } => {
                let client = Client::connect(database_url, NoTls).map_err(|source| {
                    RuntimeStateError::Postgres {
                        operation: "connect postgres database",
                        source,
                    }
                })?;
                RuntimeStorageConnection::Postgres {
                    database_url: database_url.clone(),
                    client,
                }
            }
        };

        let mut store = Self { connection };
        store.run_migrations(now_ms)?;
        Ok(store)
    }

    pub fn inspect_serve_lease(
        &mut self,
        now_ms: i64,
    ) -> Result<ServeLeaseSnapshot, RuntimeStateError> {
        let status = self.inspect_daemon_status(now_ms)?;
        Ok(ServeLeaseSnapshot {
            state: status.state,
            owner_id: status.owner_id,
            expires_at_ms: status.lease_expires_at_ms,
        })
    }

    pub fn inspect_daemon_status(
        &mut self,
        now_ms: i64,
    ) -> Result<RuntimeDaemonStatus, RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let lease_row = connection
                    .query_row(
                        "SELECT owner_id, expires_at_ms FROM serve_leases WHERE lease_key = ?1",
                        params![SERVE_LEASE_KEY],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .optional()
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "inspect sqlite serve lease",
                        source,
                    })?;
                let daemon_row = connection
                    .query_row(
                        "SELECT owner_id, pid, started_at_ms, last_heartbeat_at_ms,
                                lease_expires_at_ms, last_reload_at_ms, stop_requested_at_ms,
                                stopped_at_ms, last_error_code, last_error_message
                         FROM daemon_sessions WHERE lease_key = ?1",
                        params![SERVE_LEASE_KEY],
                        |row| {
                            Ok(DaemonSessionRow {
                                owner_id: row.get(0)?,
                                pid: row.get(1)?,
                                started_at_ms: row.get(2)?,
                                last_heartbeat_at_ms: row.get(3)?,
                                lease_expires_at_ms: row.get(4)?,
                                last_reload_at_ms: row.get(5)?,
                                stop_requested_at_ms: row.get(6)?,
                                stopped_at_ms: row.get(7)?,
                                last_error_code: row.get(8)?,
                                last_error_message: row.get(9)?,
                            })
                        },
                    )
                    .optional()
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "inspect sqlite daemon session",
                        source,
                    })?;
                Ok(build_runtime_daemon_status(daemon_row, lease_row, now_ms))
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let lease_row = client
                    .query_opt(
                        "SELECT owner_id, expires_at_ms FROM serve_leases WHERE lease_key = $1",
                        &[&SERVE_LEASE_KEY],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "inspect postgres serve lease",
                        source,
                    })?
                    .map(|row| (row.get::<_, String>(0), row.get::<_, i64>(1)));
                let daemon_row = client
                    .query_opt(
                        "SELECT owner_id, pid, started_at_ms, last_heartbeat_at_ms,
                                lease_expires_at_ms, last_reload_at_ms, stop_requested_at_ms,
                                stopped_at_ms, last_error_code, last_error_message
                         FROM daemon_sessions WHERE lease_key = $1",
                        &[&SERVE_LEASE_KEY],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "inspect postgres daemon session",
                        source,
                    })?
                    .map(|row| DaemonSessionRow {
                        owner_id: row.get(0),
                        pid: row.get(1),
                        started_at_ms: row.get(2),
                        last_heartbeat_at_ms: row.get(3),
                        lease_expires_at_ms: row.get(4),
                        last_reload_at_ms: row.get(5),
                        stop_requested_at_ms: row.get(6),
                        stopped_at_ms: row.get(7),
                        last_error_code: row.get(8),
                        last_error_message: row.get(9),
                    });
                Ok(build_runtime_daemon_status(daemon_row, lease_row, now_ms))
            }
        }
    }

    pub fn register_daemon_start(
        &mut self,
        owner_id: &str,
        pid: Option<i64>,
        now_ms: i64,
        lease_expires_at_ms: i64,
    ) -> Result<(), RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "INSERT INTO daemon_sessions (
                             lease_key, owner_id, pid, started_at_ms, last_heartbeat_at_ms,
                             lease_expires_at_ms, last_reload_at_ms, stop_requested_at_ms,
                             stopped_at_ms, last_error_code, last_error_message
                         ) VALUES (?1, ?2, ?3, ?4, ?4, ?5, NULL, NULL, NULL, NULL, NULL)
                         ON CONFLICT(lease_key) DO UPDATE SET
                             owner_id = excluded.owner_id,
                             pid = excluded.pid,
                             started_at_ms = excluded.started_at_ms,
                             last_heartbeat_at_ms = excluded.last_heartbeat_at_ms,
                             lease_expires_at_ms = excluded.lease_expires_at_ms,
                             last_reload_at_ms = NULL,
                             stop_requested_at_ms = NULL,
                             stopped_at_ms = NULL,
                             last_error_code = NULL,
                             last_error_message = NULL",
                        params![SERVE_LEASE_KEY, owner_id, pid, now_ms, lease_expires_at_ms],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "register sqlite daemon start",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client
                    .execute(
                        "INSERT INTO daemon_sessions (
                             lease_key, owner_id, pid, started_at_ms, last_heartbeat_at_ms,
                             lease_expires_at_ms, last_reload_at_ms, stop_requested_at_ms,
                             stopped_at_ms, last_error_code, last_error_message
                         ) VALUES ($1, $2, $3, $4, $4, $5, NULL, NULL, NULL, NULL, NULL)
                         ON CONFLICT(lease_key) DO UPDATE SET
                             owner_id = EXCLUDED.owner_id,
                             pid = EXCLUDED.pid,
                             started_at_ms = EXCLUDED.started_at_ms,
                             last_heartbeat_at_ms = EXCLUDED.last_heartbeat_at_ms,
                             lease_expires_at_ms = EXCLUDED.lease_expires_at_ms,
                             last_reload_at_ms = NULL,
                             stop_requested_at_ms = NULL,
                             stopped_at_ms = NULL,
                             last_error_code = NULL,
                             last_error_message = NULL",
                        &[
                            &SERVE_LEASE_KEY,
                            &owner_id,
                            &pid,
                            &now_ms,
                            &lease_expires_at_ms,
                        ],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "register postgres daemon start",
                        source,
                    })?;
            }
        }

        Ok(())
    }

    pub fn attach_daemon_pid(
        &mut self,
        owner_id: &str,
        pid: i64,
    ) -> Result<bool, RuntimeStateError> {
        let rows = match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => connection
                .execute(
                    "UPDATE daemon_sessions
                     SET pid = ?1
                     WHERE lease_key = ?2 AND owner_id = ?3 AND stopped_at_ms IS NULL",
                    params![pid, SERVE_LEASE_KEY, owner_id],
                )
                .map_err(|source| RuntimeStateError::Sqlite {
                    path: path.clone(),
                    operation: "attach sqlite daemon pid",
                    source,
                })?,
            RuntimeStorageConnection::Postgres { client, .. } => client
                .execute(
                    "UPDATE daemon_sessions
                     SET pid = $1
                     WHERE lease_key = $2 AND owner_id = $3 AND stopped_at_ms IS NULL",
                    &[&pid, &SERVE_LEASE_KEY, &owner_id],
                )
                .map_err(|source| RuntimeStateError::Postgres {
                    operation: "attach postgres daemon pid",
                    source,
                })? as usize,
        };

        Ok(rows > 0)
    }

    pub fn heartbeat_daemon(
        &mut self,
        owner_id: &str,
        pid: Option<i64>,
        now_ms: i64,
        lease_expires_at_ms: i64,
    ) -> Result<(), RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "UPDATE daemon_sessions
                         SET pid = ?1,
                             last_heartbeat_at_ms = ?2,
                             lease_expires_at_ms = ?3,
                             stopped_at_ms = NULL
                         WHERE lease_key = ?4 AND owner_id = ?5",
                        params![pid, now_ms, lease_expires_at_ms, SERVE_LEASE_KEY, owner_id],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "heartbeat sqlite daemon session",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client
                    .execute(
                        "UPDATE daemon_sessions
                         SET pid = $1,
                             last_heartbeat_at_ms = $2,
                             lease_expires_at_ms = $3,
                             stopped_at_ms = NULL
                         WHERE lease_key = $4 AND owner_id = $5",
                        &[
                            &pid,
                            &now_ms,
                            &lease_expires_at_ms,
                            &SERVE_LEASE_KEY,
                            &owner_id,
                        ],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "heartbeat postgres daemon session",
                        source,
                    })?;
            }
        }

        Ok(())
    }

    pub fn mark_daemon_reload(
        &mut self,
        owner_id: &str,
        now_ms: i64,
    ) -> Result<(), RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "UPDATE daemon_sessions
                         SET last_reload_at_ms = ?1,
                             last_error_code = NULL,
                             last_error_message = NULL
                         WHERE lease_key = ?2 AND owner_id = ?3",
                        params![now_ms, SERVE_LEASE_KEY, owner_id],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "mark sqlite daemon reload",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client
                    .execute(
                        "UPDATE daemon_sessions
                         SET last_reload_at_ms = $1,
                             last_error_code = NULL,
                             last_error_message = NULL
                         WHERE lease_key = $2 AND owner_id = $3",
                        &[&now_ms, &SERVE_LEASE_KEY, &owner_id],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "mark postgres daemon reload",
                        source,
                    })?;
            }
        }

        Ok(())
    }

    pub fn record_daemon_error(
        &mut self,
        owner_id: &str,
        error_code: &str,
        error_message: &str,
    ) -> Result<(), RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "UPDATE daemon_sessions
                         SET last_error_code = ?1,
                             last_error_message = ?2
                         WHERE lease_key = ?3 AND owner_id = ?4",
                        params![error_code, error_message, SERVE_LEASE_KEY, owner_id],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "record sqlite daemon error",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client
                    .execute(
                        "UPDATE daemon_sessions
                         SET last_error_code = $1,
                             last_error_message = $2
                         WHERE lease_key = $3 AND owner_id = $4",
                        &[&error_code, &error_message, &SERVE_LEASE_KEY, &owner_id],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "record postgres daemon error",
                        source,
                    })?;
            }
        }

        Ok(())
    }

    pub fn request_daemon_stop(&mut self, now_ms: i64) -> Result<bool, RuntimeStateError> {
        let rows = match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => connection
                .execute(
                    "UPDATE daemon_sessions
                     SET stop_requested_at_ms = COALESCE(stop_requested_at_ms, ?1)
                     WHERE lease_key = ?2 AND stopped_at_ms IS NULL",
                    params![now_ms, SERVE_LEASE_KEY],
                )
                .map_err(|source| RuntimeStateError::Sqlite {
                    path: path.clone(),
                    operation: "request sqlite daemon stop",
                    source,
                })?,
            RuntimeStorageConnection::Postgres { client, .. } => client
                .execute(
                    "UPDATE daemon_sessions
                     SET stop_requested_at_ms = COALESCE(stop_requested_at_ms, $1)
                     WHERE lease_key = $2 AND stopped_at_ms IS NULL",
                    &[&now_ms, &SERVE_LEASE_KEY],
                )
                .map_err(|source| RuntimeStateError::Postgres {
                    operation: "request postgres daemon stop",
                    source,
                })? as usize,
        };
        Ok(rows > 0)
    }

    pub fn daemon_stop_requested(&mut self, owner_id: &str) -> Result<bool, RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let requested = connection
                    .query_row(
                        "SELECT stop_requested_at_ms FROM daemon_sessions
                         WHERE lease_key = ?1 AND owner_id = ?2 AND stopped_at_ms IS NULL",
                        params![SERVE_LEASE_KEY, owner_id],
                        |row| row.get::<_, Option<i64>>(0),
                    )
                    .optional()
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "inspect sqlite daemon stop request",
                        source,
                    })?
                    .flatten();
                Ok(requested.is_some())
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let requested = client
                    .query_opt(
                        "SELECT stop_requested_at_ms FROM daemon_sessions
                         WHERE lease_key = $1 AND owner_id = $2 AND stopped_at_ms IS NULL",
                        &[&SERVE_LEASE_KEY, &owner_id],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "inspect postgres daemon stop request",
                        source,
                    })?
                    .and_then(|row| row.get::<_, Option<i64>>(0));
                Ok(requested.is_some())
            }
        }
    }

    pub fn mark_daemon_stopped(
        &mut self,
        owner_id: &str,
        now_ms: i64,
    ) -> Result<(), RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "UPDATE daemon_sessions
                         SET stopped_at_ms = ?1,
                             lease_expires_at_ms = ?1,
                             last_heartbeat_at_ms = ?1
                         WHERE lease_key = ?2 AND owner_id = ?3",
                        params![now_ms, SERVE_LEASE_KEY, owner_id],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "mark sqlite daemon stopped",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client
                    .execute(
                        "UPDATE daemon_sessions
                         SET stopped_at_ms = $1,
                             lease_expires_at_ms = $1,
                             last_heartbeat_at_ms = $1
                         WHERE lease_key = $2 AND owner_id = $3",
                        &[&now_ms, &SERVE_LEASE_KEY, &owner_id],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "mark postgres daemon stopped",
                        source,
                    })?;
            }
        }

        Ok(())
    }

    pub fn try_acquire_serve_lease(
        &mut self,
        owner_id: &str,
        now_ms: i64,
        lease_ttl_ms: i64,
    ) -> Result<LeaseAcquireResult, RuntimeStateError> {
        let expires_at_ms = now_ms.saturating_add(lease_ttl_ms.max(1));
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let transaction = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "start sqlite serve lease transaction",
                        source,
                    })?;

                let current = transaction
                    .query_row(
                        "SELECT owner_id, expires_at_ms, generation FROM serve_leases WHERE lease_key = ?1",
                        params![SERVE_LEASE_KEY],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, i64>(2)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "read current sqlite serve lease",
                        source,
                    })?;

                let takeover_generation = current
                    .as_ref()
                    .map_or(0_i64, |(_, _, generation)| generation.saturating_add(1));
                let result = match current {
                    Some((current_owner, current_expires_at_ms, _))
                        if current_expires_at_ms > now_ms && current_owner != owner_id =>
                    {
                        LeaseAcquireResult::Rejected {
                            current_owner,
                            expires_at_ms: current_expires_at_ms,
                        }
                    }
                    Some((current_owner, current_expires_at_ms, generation))
                        if current_expires_at_ms > now_ms && current_owner == owner_id =>
                    {
                        transaction
                            .execute(
                                "INSERT INTO serve_leases (lease_key, owner_id, acquired_at_ms, expires_at_ms, generation)
                                 VALUES (?1, ?2, ?3, ?4, ?5)
                                 ON CONFLICT(lease_key)
                                 DO UPDATE SET owner_id = excluded.owner_id,
                                               acquired_at_ms = excluded.acquired_at_ms,
                                               expires_at_ms = excluded.expires_at_ms,
                                               generation = excluded.generation",
                                params![SERVE_LEASE_KEY, owner_id, now_ms, expires_at_ms, generation],
                            )
                            .map_err(|source| RuntimeStateError::Sqlite {
                                path: path.clone(),
                                operation: "upsert sqlite renewed serve lease",
                                source,
                            })?;
                        LeaseAcquireResult::Renewed {
                            grant: ServeLeaseGrant {
                                owner_id: owner_id.to_owned(),
                                generation: u64::try_from(generation).unwrap_or(0),
                                expires_at_ms,
                            },
                        }
                    }
                    _ => {
                        transaction
                            .execute(
                                "INSERT INTO serve_leases (lease_key, owner_id, acquired_at_ms, expires_at_ms, generation)
                                 VALUES (?1, ?2, ?3, ?4, ?5)
                                 ON CONFLICT(lease_key)
                                 DO UPDATE SET owner_id = excluded.owner_id,
                                               acquired_at_ms = excluded.acquired_at_ms,
                                               expires_at_ms = excluded.expires_at_ms,
                                               generation = excluded.generation",
                                params![SERVE_LEASE_KEY, owner_id, now_ms, expires_at_ms, takeover_generation],
                            )
                            .map_err(|source| RuntimeStateError::Sqlite {
                                path: path.clone(),
                                operation: "upsert sqlite acquired serve lease",
                                source,
                            })?;
                        LeaseAcquireResult::Acquired {
                            grant: ServeLeaseGrant {
                                owner_id: owner_id.to_owned(),
                                generation: u64::try_from(takeover_generation).unwrap_or(0),
                                expires_at_ms,
                            },
                        }
                    }
                };

                transaction
                    .commit()
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "commit sqlite serve lease transaction",
                        source,
                    })?;

                Ok(result)
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                for _ in 0..4 {
                    let current = client
                        .query_opt(
                            "SELECT owner_id, expires_at_ms, generation FROM serve_leases WHERE lease_key = $1",
                            &[&SERVE_LEASE_KEY],
                        )
                        .map_err(|source| RuntimeStateError::Postgres {
                            operation: "read current postgres serve lease",
                            source,
                        })?
                        .map(|row| {
                            (
                                row.get::<_, String>(0),
                                row.get::<_, i64>(1),
                                row.get::<_, i64>(2),
                            )
                        });

                    if let Some((current_owner, current_expires_at_ms, generation)) = current {
                        if current_expires_at_ms > now_ms && current_owner != owner_id {
                            return Ok(LeaseAcquireResult::Rejected {
                                current_owner,
                                expires_at_ms: current_expires_at_ms,
                            });
                        }

                        let next_generation = if current_expires_at_ms > now_ms
                            && current_owner == owner_id
                        {
                            generation
                        } else {
                            generation.saturating_add(1)
                        };
                        let updated = client
                            .query_opt(
                                "UPDATE serve_leases
                                 SET owner_id = $1,
                                     acquired_at_ms = $2,
                                     expires_at_ms = $3,
                                     generation = $4
                                 WHERE lease_key = $5
                                   AND (owner_id = $1 OR expires_at_ms <= $2)
                                 RETURNING generation",
                                &[&owner_id, &now_ms, &expires_at_ms, &next_generation, &SERVE_LEASE_KEY],
                            )
                            .map_err(|source| RuntimeStateError::Postgres {
                                operation: "update postgres serve lease",
                                source,
                            })?;
                        if let Some(row) = updated {
                            let generation = row.get::<_, i64>(0);
                            return Ok(
                                if current_expires_at_ms > now_ms && current_owner == owner_id {
                                    LeaseAcquireResult::Renewed {
                                        grant: ServeLeaseGrant {
                                            owner_id: owner_id.to_owned(),
                                            generation: u64::try_from(generation).unwrap_or(0),
                                            expires_at_ms,
                                        },
                                    }
                                } else {
                                    LeaseAcquireResult::Acquired {
                                        grant: ServeLeaseGrant {
                                            owner_id: owner_id.to_owned(),
                                            generation: u64::try_from(generation).unwrap_or(0),
                                            expires_at_ms,
                                        },
                                    }
                                },
                            );
                        }

                        continue;
                    }

                    let inserted = client
                        .execute(
                            "INSERT INTO serve_leases (lease_key, owner_id, acquired_at_ms, expires_at_ms, generation)
                             VALUES ($1, $2, $3, $4, $5)
                             ON CONFLICT(lease_key) DO NOTHING",
                            &[&SERVE_LEASE_KEY, &owner_id, &now_ms, &expires_at_ms, &0_i64],
                        )
                        .map_err(|source| RuntimeStateError::Postgres {
                            operation: "insert postgres serve lease",
                            source,
                        })?;
                    if inserted > 0 {
                        return Ok(LeaseAcquireResult::Acquired {
                            grant: ServeLeaseGrant {
                                owner_id: owner_id.to_owned(),
                                generation: 0,
                                expires_at_ms,
                            },
                        });
                    }
                }

                let inserted = client
                    .execute(
                        "INSERT INTO serve_leases (lease_key, owner_id, acquired_at_ms, expires_at_ms)
                         VALUES ($1, $2, $3, $4)
                         ON CONFLICT(lease_key) DO NOTHING",
                        &[&SERVE_LEASE_KEY, &owner_id, &now_ms, &expires_at_ms],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "final insert postgres serve lease",
                        source,
                    })?;
                if inserted > 0 {
                    return Ok(LeaseAcquireResult::Acquired {
                        grant: ServeLeaseGrant {
                            owner_id: owner_id.to_owned(),
                            generation: 0,
                            expires_at_ms,
                        },
                    });
                }

                let current = client
                    .query_one(
                        "SELECT owner_id, expires_at_ms FROM serve_leases WHERE lease_key = $1",
                        &[&SERVE_LEASE_KEY],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "inspect conflicted postgres serve lease",
                        source,
                    })?;
                Ok(LeaseAcquireResult::Rejected {
                    current_owner: current.get(0),
                    expires_at_ms: current.get(1),
                })
            }
        }
    }

    pub fn release_serve_lease(&mut self, owner_id: &str) -> Result<bool, RuntimeStateError> {
        let rows = match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => connection
                .execute(
                    "UPDATE serve_leases
                     SET expires_at_ms = 0
                     WHERE lease_key = ?1 AND owner_id = ?2",
                    params![SERVE_LEASE_KEY, owner_id],
                )
                .map_err(|source| RuntimeStateError::Sqlite {
                    path: path.clone(),
                    operation: "release sqlite serve lease",
                    source,
                })?,
            RuntimeStorageConnection::Postgres { client, .. } => client
                .execute(
                    "UPDATE serve_leases
                     SET expires_at_ms = 0
                     WHERE lease_key = $1 AND owner_id = $2",
                    &[&SERVE_LEASE_KEY, &owner_id],
                )
                .map_err(|source| RuntimeStateError::Postgres {
                    operation: "release postgres serve lease",
                    source,
                })? as usize,
        };
        Ok(rows > 0)
    }

    pub fn list_run_summaries(&mut self) -> Result<Vec<RunRecordSummary>, RuntimeStateError> {
        let mut summaries = match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let mut statement = connection
                    .prepare(
                        "SELECT schema_version, run_id, workflow_id, status, started_at_ms, finished_at_ms,
                                owner_id, lease_generation
                         FROM run_summaries ORDER BY run_id ASC, started_at_ms ASC",
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "prepare sqlite run summaries query",
                        source,
                    })?;
                let rows = statement
                    .query_map([], |row| {
                        let status = row.get::<_, String>(3)?;
                        Ok(RunRecordSummary {
                            schema_version: row.get(0)?,
                            run_id: row.get(1)?,
                            workflow_id: row.get(2)?,
                            status: parse_run_status(&status),
                            started_at_ms: row.get(4)?,
                            finished_at_ms: row.get(5)?,
                            owner_id: row.get(6)?,
                            lease_generation: row
                                .get::<_, Option<i64>>(7)?
                                .map(|generation| u64::try_from(generation).unwrap_or(0)),
                        })
                    })
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "query sqlite run summaries",
                        source,
                    })?;
                let mut result = Vec::new();
                for row in rows {
                    result.push(row.map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "decode sqlite run summary row",
                        source,
                    })?);
                }
                result
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let rows = client
                    .query(
                        "SELECT schema_version, run_id, workflow_id, status, started_at_ms, finished_at_ms,
                                owner_id, lease_generation
                         FROM run_summaries ORDER BY run_id ASC, started_at_ms ASC",
                        &[],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "query postgres run summaries",
                        source,
                    })?;
                rows.into_iter()
                    .map(|row| RunRecordSummary {
                        schema_version: row.get(0),
                        run_id: row.get(1),
                        workflow_id: row.get(2),
                        status: parse_run_status(&row.get::<_, String>(3)),
                        started_at_ms: row.get(4),
                        finished_at_ms: row.get(5),
                        owner_id: row.get(6),
                        lease_generation: row
                            .get::<_, Option<i64>>(7)
                            .map(|generation| u64::try_from(generation).unwrap_or(0)),
                    })
                    .collect::<Vec<_>>()
            }
        };

        summaries.sort_by(|left, right| {
            left.run_id
                .cmp(&right.run_id)
                .then(left.started_at_ms.cmp(&right.started_at_ms))
        });
        Ok(summaries)
    }

    pub fn list_recent_run_summaries(
        &mut self,
        limit: usize,
    ) -> Result<Vec<RunRecordSummary>, RuntimeStateError> {
        let limit = limit.max(1).min(i64::MAX as usize);
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let mut statement = connection
                    .prepare(
                        "SELECT schema_version, run_id, workflow_id, status, started_at_ms, finished_at_ms,
                                owner_id, lease_generation
                         FROM run_summaries
                         ORDER BY started_at_ms DESC, run_id DESC
                         LIMIT ?1",
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "prepare sqlite recent run summaries query",
                        source,
                    })?;
                let rows = statement
                    .query_map(params![limit_i64], |row| {
                        let status = row.get::<_, String>(3)?;
                        Ok(RunRecordSummary {
                            schema_version: row.get(0)?,
                            run_id: row.get(1)?,
                            workflow_id: row.get(2)?,
                            status: parse_run_status(&status),
                            started_at_ms: row.get(4)?,
                            finished_at_ms: row.get(5)?,
                            owner_id: row.get(6)?,
                            lease_generation: row
                                .get::<_, Option<i64>>(7)?
                                .map(|generation| u64::try_from(generation).unwrap_or(0)),
                        })
                    })
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "query sqlite recent run summaries",
                        source,
                    })?;
                let mut result = Vec::new();
                for row in rows {
                    result.push(row.map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "decode sqlite recent run summary row",
                        source,
                    })?);
                }
                Ok(result)
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let rows = client
                    .query(
                        "SELECT schema_version, run_id, workflow_id, status, started_at_ms, finished_at_ms,
                                owner_id, lease_generation
                         FROM run_summaries
                         ORDER BY started_at_ms DESC, run_id DESC
                         LIMIT $1",
                        &[&limit_i64],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "query postgres recent run summaries",
                        source,
                    })?;
                Ok(rows
                    .into_iter()
                    .map(|row| RunRecordSummary {
                        schema_version: row.get(0),
                        run_id: row.get(1),
                        workflow_id: row.get(2),
                        status: parse_run_status(&row.get::<_, String>(3)),
                        started_at_ms: row.get(4),
                        finished_at_ms: row.get(5),
                        owner_id: row.get(6),
                        lease_generation: row
                            .get::<_, Option<i64>>(7)
                            .map(|generation| u64::try_from(generation).unwrap_or(0)),
                    })
                    .collect())
            }
        }
    }

    pub fn write_run_summary(
        &mut self,
        summary: &RunRecordSummary,
    ) -> Result<(), RuntimeStateError> {
        let status = render_run_status(summary.status);
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "INSERT INTO run_summaries (
                             schema_version, run_id, workflow_id, status, started_at_ms, finished_at_ms,
                             owner_id, lease_generation
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                         ON CONFLICT(run_id)
                         DO UPDATE SET schema_version = excluded.schema_version,
                                       workflow_id = excluded.workflow_id,
                                       status = excluded.status,
                                       started_at_ms = excluded.started_at_ms,
                                       finished_at_ms = excluded.finished_at_ms,
                                       owner_id = excluded.owner_id,
                                       lease_generation = excluded.lease_generation",
                        params![
                            summary.schema_version,
                            summary.run_id,
                            summary.workflow_id,
                            status,
                            summary.started_at_ms,
                            summary.finished_at_ms,
                            summary.owner_id,
                            summary
                                .lease_generation
                                .map(|generation| i64::try_from(generation).unwrap_or(i64::MAX))
                        ],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "upsert sqlite run summary",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client
                    .execute(
                        "INSERT INTO run_summaries (
                             schema_version, run_id, workflow_id, status, started_at_ms, finished_at_ms,
                             owner_id, lease_generation
                         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                         ON CONFLICT(run_id)
                         DO UPDATE SET schema_version = EXCLUDED.schema_version,
                                       workflow_id = EXCLUDED.workflow_id,
                                       status = EXCLUDED.status,
                                       started_at_ms = EXCLUDED.started_at_ms,
                                       finished_at_ms = EXCLUDED.finished_at_ms,
                                       owner_id = EXCLUDED.owner_id,
                                       lease_generation = EXCLUDED.lease_generation",
                        &[
                            &summary.schema_version,
                            &summary.run_id,
                            &summary.workflow_id,
                            &status,
                            &summary.started_at_ms,
                            &summary.finished_at_ms,
                            &summary.owner_id,
                            &summary
                                .lease_generation
                                .map(|generation| i64::try_from(generation).unwrap_or(i64::MAX)),
                        ],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "upsert postgres run summary",
                        source,
                    })?;
            }
        }
        Ok(())
    }

    pub fn write_fenced_terminal_run_summary(
        &mut self,
        summary: &RunRecordSummary,
        now_ms: i64,
    ) -> Result<bool, RuntimeStateError> {
        let (Some(owner_id), Some(lease_generation)) =
            (summary.owner_id.as_deref(), summary.lease_generation)
        else {
            return self.write_run_summary(summary).map(|_| true);
        };
        let status = render_run_status(summary.status);
        let lease_generation = i64::try_from(lease_generation).unwrap_or(i64::MAX);
        let rows = match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => connection
                .execute(
                    "UPDATE run_summaries
                     SET schema_version = ?1, workflow_id = ?2, status = ?3,
                         started_at_ms = ?4, finished_at_ms = ?5
                     WHERE run_id = ?6 AND owner_id = ?7 AND lease_generation = ?8
                       AND EXISTS (
                           SELECT 1 FROM serve_leases
                           WHERE lease_key = ?9 AND owner_id = ?7 AND generation = ?8
                             AND expires_at_ms > ?10
                       )",
                    params![
                        summary.schema_version,
                        summary.workflow_id,
                        status,
                        summary.started_at_ms,
                        summary.finished_at_ms,
                        summary.run_id,
                        owner_id,
                        lease_generation,
                        SERVE_LEASE_KEY,
                        now_ms,
                    ],
                )
                .map_err(|source| RuntimeStateError::Sqlite {
                    path: path.clone(),
                    operation: "conditionally finalize sqlite fenced run summary",
                    source,
                })?,
            RuntimeStorageConnection::Postgres { client, .. } => client
                .execute(
                    "UPDATE run_summaries
                     SET schema_version = $1, workflow_id = $2, status = $3,
                         started_at_ms = $4, finished_at_ms = $5
                     WHERE run_id = $6 AND owner_id = $7 AND lease_generation = $8
                       AND EXISTS (
                           SELECT 1 FROM serve_leases
                           WHERE lease_key = $9 AND owner_id = $7 AND generation = $8
                             AND expires_at_ms > $10
                       )",
                    &[
                        &summary.schema_version,
                        &summary.workflow_id,
                        &status,
                        &summary.started_at_ms,
                        &summary.finished_at_ms,
                        &summary.run_id,
                        &owner_id,
                        &lease_generation,
                        &SERVE_LEASE_KEY,
                        &now_ms,
                    ],
                )
                .map_err(|source| RuntimeStateError::Postgres {
                    operation: "conditionally finalize postgres fenced run summary",
                    source,
                })? as usize,
        };
        Ok(rows > 0)
    }

    pub fn append_workflow_log_entry(
        &mut self,
        run_id: &str,
        event: &str,
        message: &str,
        occurred_at_ms: i64,
    ) -> Result<u64, RuntimeStateError> {
        let next_sequence = self.next_workflow_log_sequence(run_id)?;
        let sequence_i64 = i64::try_from(next_sequence).unwrap_or(i64::MAX);
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "INSERT INTO workflow_runtime_logs (run_id, sequence, event, message, occurred_at_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![run_id, sequence_i64, event, message, occurred_at_ms],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "insert sqlite workflow runtime log",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client
                    .execute(
                        "INSERT INTO workflow_runtime_logs (run_id, sequence, event, message, occurred_at_ms)
                         VALUES ($1, $2, $3, $4, $5)",
                        &[&run_id, &sequence_i64, &event, &message, &occurred_at_ms],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "insert postgres workflow runtime log",
                        source,
                    })?;
            }
        }
        Ok(next_sequence)
    }

    pub fn list_recent_workflow_log_entries(
        &mut self,
        limit: usize,
        run_id: Option<&str>,
    ) -> Result<Vec<WorkflowRuntimeLogEntry>, RuntimeStateError> {
        let limit = limit.max(1).min(i64::MAX as usize);
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let mut result = Vec::new();
                if let Some(run_id) = run_id {
                    let mut statement = connection
                        .prepare(
                            "SELECT run_id, sequence, event, message, occurred_at_ms
                             FROM workflow_runtime_logs
                             WHERE run_id = ?1
                             ORDER BY sequence DESC
                             LIMIT ?2",
                        )
                        .map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "prepare sqlite workflow logs query",
                            source,
                        })?;
                    let rows = statement
                        .query_map(params![run_id, limit_i64], |row| {
                            Ok(WorkflowRuntimeLogEntry {
                                run_id: row.get(0)?,
                                sequence: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                                event: row.get(2)?,
                                message: row.get(3)?,
                                occurred_at_ms: row.get(4)?,
                            })
                        })
                        .map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "query sqlite workflow logs",
                            source,
                        })?;
                    for row in rows {
                        result.push(row.map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "decode sqlite workflow log row",
                            source,
                        })?);
                    }
                } else {
                    let mut statement = connection
                        .prepare(
                            "SELECT run_id, sequence, event, message, occurred_at_ms
                             FROM workflow_runtime_logs
                             ORDER BY occurred_at_ms DESC, run_id DESC, sequence DESC
                             LIMIT ?1",
                        )
                        .map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "prepare sqlite workflow logs query",
                            source,
                        })?;
                    let rows = statement
                        .query_map(params![limit_i64], |row| {
                            Ok(WorkflowRuntimeLogEntry {
                                run_id: row.get(0)?,
                                sequence: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
                                event: row.get(2)?,
                                message: row.get(3)?,
                                occurred_at_ms: row.get(4)?,
                            })
                        })
                        .map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "query sqlite workflow logs",
                            source,
                        })?;
                    for row in rows {
                        result.push(row.map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "decode sqlite workflow log row",
                            source,
                        })?);
                    }
                }
                Ok(result)
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let rows = match run_id {
                    Some(run_id) => client.query(
                        "SELECT run_id, sequence, event, message, occurred_at_ms
                         FROM workflow_runtime_logs
                         WHERE run_id = $1
                         ORDER BY sequence DESC
                         LIMIT $2",
                        &[&run_id, &limit_i64],
                    ),
                    None => client.query(
                        "SELECT run_id, sequence, event, message, occurred_at_ms
                         FROM workflow_runtime_logs
                         ORDER BY occurred_at_ms DESC, run_id DESC, sequence DESC
                         LIMIT $1",
                        &[&limit_i64],
                    ),
                }
                .map_err(|source| RuntimeStateError::Postgres {
                    operation: "query postgres workflow logs",
                    source,
                })?;
                Ok(rows
                    .into_iter()
                    .map(|row| WorkflowRuntimeLogEntry {
                        run_id: row.get(0),
                        sequence: u64::try_from(row.get::<_, i64>(1)).unwrap_or(0),
                        event: row.get(2),
                        message: row.get(3),
                        occurred_at_ms: row.get(4),
                    })
                    .collect())
            }
        }
    }

    pub fn list_trigger_snapshots(
        &mut self,
    ) -> Result<Vec<TriggerSnapshotRecord>, RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let mut statement = connection
                    .prepare(
                        "SELECT schema_version, trigger_id, last_event_id, last_accepted_at_ms, last_sequence,
                                accepted_event_ids_json, dedup_tokens_json, cooldown_tokens_json
                         FROM trigger_snapshots ORDER BY trigger_id ASC",
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "prepare sqlite trigger snapshots query",
                        source,
                    })?;
                let rows = statement
                    .query_map([], |row| {
                        let accepted_json: String = row.get(5)?;
                        let dedup_json: String = row.get(6)?;
                        let cooldown_json: String = row.get(7)?;
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                            row.get::<_, i64>(4)?,
                            accepted_json,
                            dedup_json,
                            cooldown_json,
                        ))
                    })
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "query sqlite trigger snapshots",
                        source,
                    })?;

                let mut snapshots = Vec::new();
                for row in rows {
                    let (
                        schema_version,
                        trigger_id,
                        last_event_id,
                        last_accepted_at_ms,
                        last_sequence,
                        accepted_json,
                        dedup_json,
                        cooldown_json,
                    ) = row.map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "decode sqlite trigger snapshot row",
                        source,
                    })?;
                    snapshots.push(decode_snapshot(
                        schema_version,
                        trigger_id,
                        last_event_id,
                        last_accepted_at_ms,
                        last_sequence,
                        &accepted_json,
                        &dedup_json,
                        &cooldown_json,
                    )?);
                }
                Ok(snapshots)
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let rows = client
                    .query(
                        "SELECT schema_version, trigger_id, last_event_id, last_accepted_at_ms, last_sequence,
                                accepted_event_ids_json, dedup_tokens_json, cooldown_tokens_json
                         FROM trigger_snapshots ORDER BY trigger_id ASC",
                        &[],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "query postgres trigger snapshots",
                        source,
                    })?;
                let mut snapshots = Vec::with_capacity(rows.len());
                for row in rows {
                    snapshots.push(decode_snapshot(
                        row.get(0),
                        row.get(1),
                        row.get(2),
                        row.get(3),
                        row.get(4),
                        &row.get::<_, String>(5),
                        &row.get::<_, String>(6),
                        &row.get::<_, String>(7),
                    )?);
                }
                Ok(snapshots)
            }
        }
    }

    pub fn read_trigger_snapshot(
        &mut self,
        trigger_id: &str,
    ) -> Result<Option<TriggerSnapshotRecord>, RuntimeStateError> {
        let snapshots = self.list_trigger_snapshots()?;
        Ok(snapshots
            .into_iter()
            .find(|snapshot| snapshot.trigger_id == trigger_id))
    }

    pub fn write_trigger_snapshot(
        &mut self,
        snapshot: &TriggerSnapshotRecord,
    ) -> Result<(), RuntimeStateError> {
        let accepted_json =
            serde_json::to_string(&snapshot.accepted_event_ids).map_err(|source| {
                RuntimeStateError::JsonEncode {
                    field: "trigger_snapshots.accepted_event_ids_json",
                    source,
                }
            })?;
        let dedup_json = serde_json::to_string(&snapshot.dedup_tokens).map_err(|source| {
            RuntimeStateError::JsonEncode {
                field: "trigger_snapshots.dedup_tokens_json",
                source,
            }
        })?;
        let cooldown_json = serde_json::to_string(&snapshot.cooldown_tokens).map_err(|source| {
            RuntimeStateError::JsonEncode {
                field: "trigger_snapshots.cooldown_tokens_json",
                source,
            }
        })?;
        let last_sequence = i64::try_from(snapshot.last_sequence).unwrap_or(i64::MAX);

        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "INSERT INTO trigger_snapshots (
                            schema_version, trigger_id, last_event_id, last_accepted_at_ms, last_sequence,
                            accepted_event_ids_json, dedup_tokens_json, cooldown_tokens_json
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                         ON CONFLICT(trigger_id)
                         DO UPDATE SET
                            schema_version = excluded.schema_version,
                            last_event_id = excluded.last_event_id,
                            last_accepted_at_ms = excluded.last_accepted_at_ms,
                            last_sequence = excluded.last_sequence,
                            accepted_event_ids_json = excluded.accepted_event_ids_json,
                            dedup_tokens_json = excluded.dedup_tokens_json,
                            cooldown_tokens_json = excluded.cooldown_tokens_json",
                        params![
                            snapshot.schema_version,
                            snapshot.trigger_id,
                            snapshot.last_event_id,
                            snapshot.last_accepted_at_ms,
                            last_sequence,
                            accepted_json,
                            dedup_json,
                            cooldown_json
                        ],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "upsert sqlite trigger snapshot",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client
                    .execute(
                        "INSERT INTO trigger_snapshots (
                            schema_version, trigger_id, last_event_id, last_accepted_at_ms, last_sequence,
                            accepted_event_ids_json, dedup_tokens_json, cooldown_tokens_json
                         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                         ON CONFLICT(trigger_id)
                         DO UPDATE SET
                            schema_version = EXCLUDED.schema_version,
                            last_event_id = EXCLUDED.last_event_id,
                            last_accepted_at_ms = EXCLUDED.last_accepted_at_ms,
                            last_sequence = EXCLUDED.last_sequence,
                            accepted_event_ids_json = EXCLUDED.accepted_event_ids_json,
                            dedup_tokens_json = EXCLUDED.dedup_tokens_json,
                            cooldown_tokens_json = EXCLUDED.cooldown_tokens_json",
                        &[
                            &snapshot.schema_version,
                            &snapshot.trigger_id,
                            &snapshot.last_event_id,
                            &snapshot.last_accepted_at_ms,
                            &last_sequence,
                            &accepted_json,
                            &dedup_json,
                            &cooldown_json,
                        ],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "upsert postgres trigger snapshot",
                        source,
                    })?;
            }
        }

        Ok(())
    }

    pub fn load_trigger_records_after_sequence(
        &mut self,
        trigger_id: &str,
        sequence_exclusive: u64,
    ) -> Result<Vec<TriggerEventRecord>, RuntimeStateError> {
        let sequence_exclusive = i64::try_from(sequence_exclusive).unwrap_or(i64::MAX);
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let mut statement = connection
                    .prepare(
                        "SELECT schema_version, run_id, sequence, trigger_id, workflow_id, event_id,
                                checkpoint, source, accepted_at_ms, payload_json,
                                dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
                         FROM trigger_event_records
                         WHERE trigger_id = ?1 AND sequence > ?2
                         ORDER BY sequence ASC",
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "prepare sqlite trigger records query",
                        source,
                    })?;
                let rows = statement
                    .query_map(params![trigger_id, sequence_exclusive], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, String>(7)?,
                            row.get::<_, i64>(8)?,
                            row.get::<_, String>(9)?,
                            row.get::<_, Option<String>>(10)?,
                            row.get::<_, Option<i64>>(11)?,
                            row.get::<_, Option<String>>(12)?,
                            row.get::<_, Option<i64>>(13)?,
                        ))
                    })
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "query sqlite trigger records",
                        source,
                    })?;
                let mut records = Vec::new();
                for row in rows {
                    let row = row.map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "decode sqlite trigger record row",
                        source,
                    })?;
                    records.push(decode_trigger_record_row(row)?);
                }
                Ok(records)
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let rows = client
                    .query(
                        "SELECT schema_version, run_id, sequence, trigger_id, workflow_id, event_id,
                                checkpoint, source, accepted_at_ms, payload_json,
                                dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
                         FROM trigger_event_records
                         WHERE trigger_id = $1 AND sequence > $2
                         ORDER BY sequence ASC",
                        &[&trigger_id, &sequence_exclusive],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "query postgres trigger records",
                        source,
                    })?;
                let mut records = Vec::with_capacity(rows.len());
                for row in rows {
                    records.push(decode_trigger_record_row((
                        row.get(0),
                        row.get(1),
                        row.get(2),
                        row.get(3),
                        row.get(4),
                        row.get(5),
                        row.get(6),
                        row.get(7),
                        row.get(8),
                        row.get(9),
                        row.get(10),
                        row.get(11),
                        row.get(12),
                        row.get(13),
                    ))?);
                }
                Ok(records)
            }
        }
    }

    pub fn list_recent_trigger_records(
        &mut self,
        limit: usize,
        trigger_id: Option<&str>,
    ) -> Result<Vec<TriggerEventRecord>, RuntimeStateError> {
        let limit = limit.max(1).min(i64::MAX as usize);
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let mut result = Vec::new();
                if let Some(trigger_id) = trigger_id {
                    let mut statement = connection
                        .prepare(
                            "SELECT schema_version, run_id, sequence, trigger_id, workflow_id, event_id,
                                    checkpoint, source, accepted_at_ms, payload_json,
                                    dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
                             FROM trigger_event_records
                             WHERE trigger_id = ?1
                             ORDER BY sequence DESC
                             LIMIT ?2",
                        )
                        .map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "prepare sqlite recent trigger records query",
                            source,
                        })?;
                    let rows = statement
                        .query_map(params![trigger_id, limit_i64], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, i64>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, String>(5)?,
                                row.get::<_, Option<String>>(6)?,
                                row.get::<_, String>(7)?,
                                row.get::<_, i64>(8)?,
                                row.get::<_, String>(9)?,
                                row.get::<_, Option<String>>(10)?,
                                row.get::<_, Option<i64>>(11)?,
                                row.get::<_, Option<String>>(12)?,
                                row.get::<_, Option<i64>>(13)?,
                            ))
                        })
                        .map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "query sqlite recent trigger records",
                            source,
                        })?;
                    for row in rows {
                        result.push(decode_trigger_record_row(row.map_err(|source| {
                            RuntimeStateError::Sqlite {
                                path: path.clone(),
                                operation: "decode sqlite recent trigger record row",
                                source,
                            }
                        })?)?);
                    }
                } else {
                    let mut statement = connection
                        .prepare(
                            "SELECT schema_version, run_id, sequence, trigger_id, workflow_id, event_id,
                                    checkpoint, source, accepted_at_ms, payload_json,
                                    dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
                             FROM trigger_event_records
                             ORDER BY accepted_at_ms DESC, trigger_id DESC, sequence DESC
                             LIMIT ?1",
                        )
                        .map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "prepare sqlite recent trigger records query",
                            source,
                        })?;
                    let rows = statement
                        .query_map(params![limit_i64], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, i64>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, String>(5)?,
                                row.get::<_, Option<String>>(6)?,
                                row.get::<_, String>(7)?,
                                row.get::<_, i64>(8)?,
                                row.get::<_, String>(9)?,
                                row.get::<_, Option<String>>(10)?,
                                row.get::<_, Option<i64>>(11)?,
                                row.get::<_, Option<String>>(12)?,
                                row.get::<_, Option<i64>>(13)?,
                            ))
                        })
                        .map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "query sqlite recent trigger records",
                            source,
                        })?;
                    for row in rows {
                        result.push(decode_trigger_record_row(row.map_err(|source| {
                            RuntimeStateError::Sqlite {
                                path: path.clone(),
                                operation: "decode sqlite recent trigger record row",
                                source,
                            }
                        })?)?);
                    }
                }
                Ok(result)
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let rows = match trigger_id {
                    Some(trigger_id) => client.query(
                        "SELECT schema_version, run_id, sequence, trigger_id, workflow_id, event_id,
                                checkpoint, source, accepted_at_ms, payload_json,
                                dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
                         FROM trigger_event_records
                         WHERE trigger_id = $1
                         ORDER BY sequence DESC
                         LIMIT $2",
                        &[&trigger_id, &limit_i64],
                    ),
                    None => client.query(
                        "SELECT schema_version, run_id, sequence, trigger_id, workflow_id, event_id,
                                checkpoint, source, accepted_at_ms, payload_json,
                                dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
                         FROM trigger_event_records
                         ORDER BY accepted_at_ms DESC, trigger_id DESC, sequence DESC
                         LIMIT $1",
                        &[&limit_i64],
                    ),
                }
                .map_err(|source| RuntimeStateError::Postgres {
                    operation: "query postgres recent trigger records",
                    source,
                })?;
                let mut result = Vec::with_capacity(rows.len());
                for row in rows {
                    result.push(decode_trigger_record_row((
                        row.get(0),
                        row.get(1),
                        row.get(2),
                        row.get(3),
                        row.get(4),
                        row.get(5),
                        row.get(6),
                        row.get(7),
                        row.get(8),
                        row.get(9),
                        row.get(10),
                        row.get(11),
                        row.get(12),
                        row.get(13),
                    ))?);
                }
                Ok(result)
            }
        }
    }

    pub fn list_replayable_trigger_records(
        &mut self,
        limit: usize,
    ) -> Result<Vec<TriggerEventRecord>, RuntimeStateError> {
        let limit = limit.max(1).min(i64::MAX as usize);
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let mut statement = connection
                    .prepare(
                        "SELECT tr.schema_version, tr.run_id, tr.sequence, tr.trigger_id, tr.workflow_id, tr.event_id,
                                tr.checkpoint, tr.source, tr.accepted_at_ms, tr.payload_json,
                                tr.dedup_key, tr.dedup_expires_at_ms, tr.cooldown_key, tr.cooldown_expires_at_ms
                         FROM trigger_event_records tr
                         LEFT JOIN run_summaries rs ON rs.run_id = tr.run_id
                         WHERE rs.run_id IS NULL
                         ORDER BY tr.accepted_at_ms ASC, tr.trigger_id ASC, tr.sequence ASC
                         LIMIT ?1",
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "prepare sqlite replayable trigger records query",
                        source,
                    })?;
                let rows = statement
                    .query_map(params![limit_i64], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, String>(7)?,
                            row.get::<_, i64>(8)?,
                            row.get::<_, String>(9)?,
                            row.get::<_, Option<String>>(10)?,
                            row.get::<_, Option<i64>>(11)?,
                            row.get::<_, Option<String>>(12)?,
                            row.get::<_, Option<i64>>(13)?,
                        ))
                    })
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "query sqlite replayable trigger records",
                        source,
                    })?;
                let mut records = Vec::new();
                for row in rows {
                    let row = row.map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "decode sqlite replayable trigger record row",
                        source,
                    })?;
                    records.push(decode_trigger_record_row(row)?);
                }
                Ok(records)
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let rows = client
                    .query(
                        "SELECT tr.schema_version, tr.run_id, tr.sequence, tr.trigger_id, tr.workflow_id, tr.event_id,
                                tr.checkpoint, tr.source, tr.accepted_at_ms, tr.payload_json,
                                tr.dedup_key, tr.dedup_expires_at_ms, tr.cooldown_key, tr.cooldown_expires_at_ms
                         FROM trigger_event_records tr
                         LEFT JOIN run_summaries rs ON rs.run_id = tr.run_id
                         WHERE rs.run_id IS NULL
                         ORDER BY tr.accepted_at_ms ASC, tr.trigger_id ASC, tr.sequence ASC
                         LIMIT $1",
                        &[&limit_i64],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "query postgres replayable trigger records",
                        source,
                    })?;
                let mut records = Vec::with_capacity(rows.len());
                for row in rows {
                    records.push(decode_trigger_record_row((
                        row.get(0),
                        row.get(1),
                        row.get(2),
                        row.get(3),
                        row.get(4),
                        row.get(5),
                        row.get(6),
                        row.get(7),
                        row.get(8),
                        row.get(9),
                        row.get(10),
                        row.get(11),
                        row.get(12),
                        row.get(13),
                    ))?);
                }
                Ok(records)
            }
        }
    }

    pub fn append_ingress_inbox_record(
        &mut self,
        record: &IngressInboxRecord,
    ) -> Result<bool, RuntimeStateError> {
        let payload_json = serde_json::to_string(&record.payload).map_err(|source| {
            RuntimeStateError::JsonEncode {
                field: "ingress_inbox_records.payload_json",
                source,
            }
        })?;
        let headers_json = serde_json::to_string(&record.headers).map_err(|source| {
            RuntimeStateError::JsonEncode {
                field: "ingress_inbox_records.headers_json",
                source,
            }
        })?;

        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => connection
                .execute(
                    "INSERT INTO ingress_inbox_records (
                         inbox_id, schema_version, trigger_id, workflow_id, transport_kind,
                         ingress_event_id, source, route_path, http_method, received_at_ms,
                         payload_json, headers_json, remote_addr, processed_at_ms, last_error
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                     ON CONFLICT(trigger_id, ingress_event_id) DO NOTHING",
                    params![
                        record.inbox_id,
                        record.schema_version,
                        record.trigger_id,
                        record.workflow_id,
                        record.transport_kind,
                        record.ingress_event_id,
                        record.source,
                        record.route_path,
                        record.http_method,
                        record.received_at_ms,
                        payload_json,
                        headers_json,
                        record.remote_addr,
                        record.processed_at_ms,
                        record.last_error,
                    ],
                )
                .map(|changed| changed > 0)
                .map_err(|source| RuntimeStateError::Sqlite {
                    path: path.clone(),
                    operation: "insert sqlite ingress inbox record",
                    source,
                }),
            RuntimeStorageConnection::Postgres { client, .. } => client
                .execute(
                    "INSERT INTO ingress_inbox_records (
                         inbox_id, schema_version, trigger_id, workflow_id, transport_kind,
                         ingress_event_id, source, route_path, http_method, received_at_ms,
                         payload_json, headers_json, remote_addr, processed_at_ms, last_error
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
                     ON CONFLICT(trigger_id, ingress_event_id) DO NOTHING",
                    &[
                        &record.inbox_id,
                        &record.schema_version,
                        &record.trigger_id,
                        &record.workflow_id,
                        &record.transport_kind,
                        &record.ingress_event_id,
                        &record.source,
                        &record.route_path,
                        &record.http_method,
                        &record.received_at_ms,
                        &payload_json,
                        &headers_json,
                        &record.remote_addr,
                        &record.processed_at_ms,
                        &record.last_error,
                    ],
                )
                .map(|changed| changed > 0)
                .map_err(|source| RuntimeStateError::Postgres {
                    operation: "insert postgres ingress inbox record",
                    source,
                }),
        }
    }

    pub fn list_pending_ingress_inbox_records(
        &mut self,
        trigger_id: &str,
        limit: i64,
    ) -> Result<Vec<IngressInboxRecord>, RuntimeStateError> {
        let limit = limit.max(1);
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let mut statement = connection
                    .prepare(
                        "SELECT inbox_id, schema_version, trigger_id, workflow_id, transport_kind,
                                ingress_event_id, source, route_path, http_method, received_at_ms,
                                payload_json, headers_json, remote_addr, processed_at_ms, last_error
                         FROM ingress_inbox_records
                         WHERE trigger_id = ?1 AND processed_at_ms IS NULL
                         ORDER BY received_at_ms ASC, inbox_id ASC
                         LIMIT ?2",
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "prepare sqlite pending ingress inbox query",
                        source,
                    })?;
                let rows = statement
                    .query_map(params![trigger_id, limit], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                            row.get::<_, Option<String>>(8)?,
                            row.get::<_, i64>(9)?,
                            row.get::<_, String>(10)?,
                            row.get::<_, String>(11)?,
                            row.get::<_, Option<String>>(12)?,
                            row.get::<_, Option<i64>>(13)?,
                            row.get::<_, Option<String>>(14)?,
                        ))
                    })
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "query sqlite pending ingress inbox records",
                        source,
                    })?;
                let mut records = Vec::new();
                for row in rows {
                    let row = row.map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "decode sqlite ingress inbox row",
                        source,
                    })?;
                    records.push(decode_ingress_inbox_record_row(row)?);
                }
                Ok(records)
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let rows = client
                    .query(
                        "SELECT inbox_id, schema_version, trigger_id, workflow_id, transport_kind,
                                ingress_event_id, source, route_path, http_method, received_at_ms,
                                payload_json, headers_json, remote_addr, processed_at_ms, last_error
                         FROM ingress_inbox_records
                         WHERE trigger_id = $1 AND processed_at_ms IS NULL
                         ORDER BY received_at_ms ASC, inbox_id ASC
                         LIMIT $2",
                        &[&trigger_id, &limit],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "query postgres pending ingress inbox records",
                        source,
                    })?;
                let mut records = Vec::with_capacity(rows.len());
                for row in rows {
                    records.push(decode_ingress_inbox_record_row((
                        row.get(0),
                        row.get(1),
                        row.get(2),
                        row.get(3),
                        row.get(4),
                        row.get(5),
                        row.get(6),
                        row.get(7),
                        row.get(8),
                        row.get(9),
                        row.get(10),
                        row.get(11),
                        row.get(12),
                        row.get(13),
                        row.get(14),
                    ))?);
                }
                Ok(records)
            }
        }
    }

    pub fn mark_ingress_inbox_processed(
        &mut self,
        inbox_id: &str,
        processed_at_ms: i64,
    ) -> Result<(), RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "UPDATE ingress_inbox_records
                         SET processed_at_ms = ?2, last_error = NULL
                         WHERE inbox_id = ?1",
                        params![inbox_id, processed_at_ms],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "mark sqlite ingress inbox record processed",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client
                    .execute(
                        "UPDATE ingress_inbox_records
                         SET processed_at_ms = $2, last_error = NULL
                         WHERE inbox_id = $1",
                        &[&inbox_id, &processed_at_ms],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "mark postgres ingress inbox record processed",
                        source,
                    })?;
            }
        }
        Ok(())
    }

    pub fn append_staged_trigger_event_record(
        &mut self,
        record: &StagedTriggerEventRecord,
    ) -> Result<bool, RuntimeStateError> {
        let payload_json = serde_json::to_string(&record.payload).map_err(|source| {
            RuntimeStateError::JsonEncode {
                field: "staged_trigger_event_records.payload_json",
                source,
            }
        })?;

        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => connection
                .execute(
                    "INSERT INTO staged_trigger_event_records (
                         staging_id, schema_version, trigger_id, workflow_id, event_id, source,
                         occurred_at_ms, staged_at_ms, checkpoint, payload_json,
                         dedup_key, dedup_window_ms, cooldown_key, cooldown_ms,
                         accepted_at_ms, last_error
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                     ON CONFLICT(trigger_id, event_id) DO NOTHING",
                    params![
                        record.staging_id,
                        record.schema_version,
                        record.trigger_id,
                        record.workflow_id,
                        record.event_id,
                        record.source,
                        record.occurred_at_ms,
                        record.staged_at_ms,
                        record.checkpoint,
                        payload_json,
                        record.dedup_key,
                        record.dedup_window_ms,
                        record.cooldown_key,
                        record.cooldown_ms,
                        record.accepted_at_ms,
                        record.last_error,
                    ],
                )
                .map(|changed| changed > 0)
                .map_err(|source| RuntimeStateError::Sqlite {
                    path: path.clone(),
                    operation: "insert sqlite staged trigger event record",
                    source,
                }),
            RuntimeStorageConnection::Postgres { client, .. } => client
                .execute(
                    "INSERT INTO staged_trigger_event_records (
                         staging_id, schema_version, trigger_id, workflow_id, event_id, source,
                         occurred_at_ms, staged_at_ms, checkpoint, payload_json,
                         dedup_key, dedup_window_ms, cooldown_key, cooldown_ms,
                         accepted_at_ms, last_error
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
                     ON CONFLICT(trigger_id, event_id) DO NOTHING",
                    &[
                        &record.staging_id,
                        &record.schema_version,
                        &record.trigger_id,
                        &record.workflow_id,
                        &record.event_id,
                        &record.source,
                        &record.occurred_at_ms,
                        &record.staged_at_ms,
                        &record.checkpoint,
                        &payload_json,
                        &record.dedup_key,
                        &record.dedup_window_ms,
                        &record.cooldown_key,
                        &record.cooldown_ms,
                        &record.accepted_at_ms,
                        &record.last_error,
                    ],
                )
                .map(|changed| changed > 0)
                .map_err(|source| RuntimeStateError::Postgres {
                    operation: "insert postgres staged trigger event record",
                    source,
                }),
        }
    }

    pub fn list_pending_staged_trigger_event_records(
        &mut self,
        trigger_id: &str,
        limit: i64,
    ) -> Result<Vec<StagedTriggerEventRecord>, RuntimeStateError> {
        let limit = limit.max(1);
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let mut statement = connection
                    .prepare(
                        "SELECT staging_id, schema_version, trigger_id, workflow_id, event_id, source,
                                occurred_at_ms, staged_at_ms, checkpoint, payload_json,
                                dedup_key, dedup_window_ms, cooldown_key, cooldown_ms,
                                accepted_at_ms, last_error
                         FROM staged_trigger_event_records
                         WHERE trigger_id = ?1 AND accepted_at_ms IS NULL
                         ORDER BY staged_at_ms ASC, staging_id ASC
                         LIMIT ?2",
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "prepare sqlite pending staged trigger event records query",
                        source,
                    })?;
                let rows = statement
                    .query_map(params![trigger_id, limit], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, i64>(7)?,
                            row.get::<_, Option<String>>(8)?,
                            row.get::<_, String>(9)?,
                            row.get::<_, Option<String>>(10)?,
                            row.get::<_, Option<i64>>(11)?,
                            row.get::<_, Option<String>>(12)?,
                            row.get::<_, Option<i64>>(13)?,
                            row.get::<_, Option<i64>>(14)?,
                            row.get::<_, Option<String>>(15)?,
                        ))
                    })
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "query sqlite pending staged trigger event records",
                        source,
                    })?;
                let mut records = Vec::new();
                for row in rows {
                    let row = row.map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "decode sqlite staged trigger event row",
                        source,
                    })?;
                    records.push(decode_staged_trigger_event_record_row(row)?);
                }
                Ok(records)
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let rows = client
                    .query(
                        "SELECT staging_id, schema_version, trigger_id, workflow_id, event_id, source,
                                occurred_at_ms, staged_at_ms, checkpoint, payload_json,
                                dedup_key, dedup_window_ms, cooldown_key, cooldown_ms,
                                accepted_at_ms, last_error
                         FROM staged_trigger_event_records
                         WHERE trigger_id = $1 AND accepted_at_ms IS NULL
                         ORDER BY staged_at_ms ASC, staging_id ASC
                         LIMIT $2",
                        &[&trigger_id, &limit],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "query postgres pending staged trigger event records",
                        source,
                    })?;
                let mut records = Vec::with_capacity(rows.len());
                for row in rows {
                    records.push(decode_staged_trigger_event_record_row((
                        row.get(0),
                        row.get(1),
                        row.get(2),
                        row.get(3),
                        row.get(4),
                        row.get(5),
                        row.get(6),
                        row.get(7),
                        row.get(8),
                        row.get(9),
                        row.get(10),
                        row.get(11),
                        row.get(12),
                        row.get(13),
                        row.get(14),
                        row.get(15),
                    ))?);
                }
                Ok(records)
            }
        }
    }

    pub fn mark_staged_trigger_event_accepted(
        &mut self,
        staging_id: &str,
        accepted_at_ms: i64,
    ) -> Result<(), RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "UPDATE staged_trigger_event_records
                         SET accepted_at_ms = ?2, last_error = NULL
                         WHERE staging_id = ?1",
                        params![staging_id, accepted_at_ms],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "mark sqlite staged trigger event accepted",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client
                    .execute(
                        "UPDATE staged_trigger_event_records
                         SET accepted_at_ms = $2, last_error = NULL
                         WHERE staging_id = $1",
                        &[&staging_id, &accepted_at_ms],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "mark postgres staged trigger event accepted",
                        source,
                    })?;
            }
        }
        Ok(())
    }

    pub fn reconcile_pending_staged_trigger_event_records(
        &mut self,
        limit: i64,
    ) -> Result<Vec<StagedTriggerEventRecord>, RuntimeStateError> {
        let limit = limit.max(1);
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let mut statement = connection
                    .prepare(
                        "SELECT staging_id, schema_version, trigger_id, workflow_id, event_id, source,
                                occurred_at_ms, staged_at_ms, checkpoint, payload_json,
                                dedup_key, dedup_window_ms, cooldown_key, cooldown_ms,
                                accepted_at_ms, last_error
                         FROM staged_trigger_event_records
                         WHERE accepted_at_ms IS NULL
                         ORDER BY staged_at_ms ASC, trigger_id ASC, staging_id ASC
                         LIMIT ?1",
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "prepare sqlite reconcile staged trigger event records query",
                        source,
                    })?;
                let rows = statement
                    .query_map(params![limit], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, i64>(7)?,
                            row.get::<_, Option<String>>(8)?,
                            row.get::<_, String>(9)?,
                            row.get::<_, Option<String>>(10)?,
                            row.get::<_, Option<i64>>(11)?,
                            row.get::<_, Option<String>>(12)?,
                            row.get::<_, Option<i64>>(13)?,
                            row.get::<_, Option<i64>>(14)?,
                            row.get::<_, Option<String>>(15)?,
                        ))
                    })
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "query sqlite reconcile staged trigger event records",
                        source,
                    })?;
                let mut records = Vec::new();
                for row in rows {
                    let row = row.map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "decode sqlite reconcile staged trigger event row",
                        source,
                    })?;
                    records.push(decode_staged_trigger_event_record_row(row)?);
                }
                Ok(records)
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let rows = client
                    .query(
                        "SELECT staging_id, schema_version, trigger_id, workflow_id, event_id, source,
                                occurred_at_ms, staged_at_ms, checkpoint, payload_json,
                                dedup_key, dedup_window_ms, cooldown_key, cooldown_ms,
                                accepted_at_ms, last_error
                         FROM staged_trigger_event_records
                         WHERE accepted_at_ms IS NULL
                         ORDER BY staged_at_ms ASC, trigger_id ASC, staging_id ASC
                         LIMIT $1",
                        &[&limit],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "query postgres reconcile staged trigger event records",
                        source,
                    })?;
                let mut records = Vec::with_capacity(rows.len());
                for row in rows {
                    records.push(decode_staged_trigger_event_record_row((
                        row.get(0),
                        row.get(1),
                        row.get(2),
                        row.get(3),
                        row.get(4),
                        row.get(5),
                        row.get(6),
                        row.get(7),
                        row.get(8),
                        row.get(9),
                        row.get(10),
                        row.get(11),
                        row.get(12),
                        row.get(13),
                        row.get(14),
                        row.get(15),
                    ))?);
                }
                Ok(records)
            }
        }
    }

    pub fn accept_trigger_event(
        &mut self,
        command: TriggerAcceptanceCommand,
    ) -> Result<TriggerAcceptanceOutcome, RuntimeStateError> {
        self.begin_trigger_acceptance_transaction()?;
        let result = self.accept_trigger_event_in_transaction(command);
        match result {
            Ok(outcome) => {
                if let Err(error) = self.commit_trigger_acceptance_transaction() {
                    let _ = self.rollback_trigger_acceptance_transaction();
                    return Err(error);
                }
                Ok(outcome)
            }
            Err(error) => {
                let _ = self.rollback_trigger_acceptance_transaction();
                Err(error)
            }
        }
    }

    fn accept_trigger_event_in_transaction(
        &mut self,
        command: TriggerAcceptanceCommand,
    ) -> Result<TriggerAcceptanceOutcome, RuntimeStateError> {
        let record = command.candidate_record;
        self.lock_trigger_acceptance_keys(&record)?;

        if let Some(existing) = self.find_trigger_record_by_event(&record.trigger_id, &record.event_id)? {
            if let Some(checkpoint) = existing.checkpoint {
                self.write_trigger_checkpoint(&TriggerCheckpointRecord {
                    schema_version: String::from("1.0.0"),
                    trigger_id: record.trigger_id.clone(),
                    checkpoint,
                    acked_at_ms: existing.accepted_at_ms,
                })?;
            }
            self.consume_staged_trigger_event(command.staged_id.as_deref(), record.accepted_at_ms)?;
            return Ok(TriggerAcceptanceOutcome::Duplicate);
        }

        if let Some(key) = record.dedup_key.as_deref()
            && !self.dedup_is_ready(key, record.accepted_at_ms)?
        {
            self.consume_staged_trigger_event(command.staged_id.as_deref(), record.accepted_at_ms)?;
            return Ok(TriggerAcceptanceOutcome::DedupSuppressed);
        }
        if let Some(key) = record.cooldown_key.as_deref()
            && !self.cooldown_is_ready(key, record.accepted_at_ms)?
        {
            self.consume_staged_trigger_event(command.staged_id.as_deref(), record.accepted_at_ms)?;
            return Ok(TriggerAcceptanceOutcome::CooldownSuppressed);
        }

        let mut snapshot = self
            .read_trigger_snapshot(&record.trigger_id)?
            .unwrap_or_else(|| TriggerSnapshotRecord::new(record.trigger_id.clone()));
        if snapshot.last_sequence != command.expected_snapshot_sequence {
            return Ok(TriggerAcceptanceOutcome::Conflict);
        }
        let record_ref = self.write_trigger_record(&record)?;
        snapshot.apply_record(&record);
        self.write_trigger_snapshot(&snapshot)?;
        if let Some(checkpoint) = record.checkpoint.clone() {
            self.write_trigger_checkpoint(&TriggerCheckpointRecord {
                schema_version: String::from("1.0.0"),
                trigger_id: record.trigger_id.clone(),
                checkpoint,
                acked_at_ms: record.accepted_at_ms,
            })?;
        }
        self.consume_staged_trigger_event(command.staged_id.as_deref(), record.accepted_at_ms)?;

        Ok(TriggerAcceptanceOutcome::Accepted {
            request: TriggerRunRequest {
                run_id: record.run_id,
                workflow_id: record.workflow_id,
                trigger_id: record.trigger_id,
                event_id: record.event_id,
                source: record.source,
                accepted_at_ms: record.accepted_at_ms,
                payload: record.payload,
                trigger_record_ref: record_ref.clone(),
            },
            record_ref,
        })
    }

    fn begin_trigger_acceptance_transaction(&mut self) -> Result<(), RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => connection
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(|source| RuntimeStateError::Sqlite {
                    path: path.clone(),
                    operation: "begin sqlite trigger acceptance transaction",
                    source,
                }),
            RuntimeStorageConnection::Postgres { client, .. } => client
                .batch_execute("BEGIN")
                .map_err(|source| RuntimeStateError::Postgres {
                    operation: "begin postgres trigger acceptance transaction",
                    source,
                }),
        }
    }

    fn commit_trigger_acceptance_transaction(&mut self) -> Result<(), RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => connection
                .execute_batch("COMMIT")
                .map_err(|source| RuntimeStateError::Sqlite {
                    path: path.clone(),
                    operation: "commit sqlite trigger acceptance transaction",
                    source,
                }),
            RuntimeStorageConnection::Postgres { client, .. } => client
                .batch_execute("COMMIT")
                .map_err(|source| RuntimeStateError::Postgres {
                    operation: "commit postgres trigger acceptance transaction",
                    source,
                }),
        }
    }

    fn rollback_trigger_acceptance_transaction(&mut self) -> Result<(), RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => connection
                .execute_batch("ROLLBACK")
                .map_err(|source| RuntimeStateError::Sqlite {
                    path: path.clone(),
                    operation: "rollback sqlite trigger acceptance transaction",
                    source,
                }),
            RuntimeStorageConnection::Postgres { client, .. } => client
                .batch_execute("ROLLBACK")
                .map_err(|source| RuntimeStateError::Postgres {
                    operation: "rollback postgres trigger acceptance transaction",
                    source,
                }),
        }
    }

    fn lock_trigger_acceptance_keys(
        &mut self,
        record: &TriggerEventRecord,
    ) -> Result<(), RuntimeStateError> {
        let mut keys = BTreeSet::from([format!("trigger:{}", record.trigger_id)]);
        if let Some(key) = record.dedup_key.as_deref() {
            keys.insert(format!("dedup:{key}"));
        }
        if let Some(key) = record.cooldown_key.as_deref() {
            keys.insert(format!("cooldown:{key}"));
        }
        if let RuntimeStorageConnection::Postgres { client, .. } = &mut self.connection {
            for key in keys {
                client
                    .query_one("SELECT pg_advisory_xact_lock(hashtext($1))", &[&key])
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "lock postgres trigger acceptance key",
                        source,
                    })?;
            }
        }
        Ok(())
    }

    fn consume_staged_trigger_event(
        &mut self,
        staged_id: Option<&str>,
        accepted_at_ms: i64,
    ) -> Result<(), RuntimeStateError> {
        if let Some(staged_id) = staged_id {
            self.mark_staged_trigger_event_accepted(staged_id, accepted_at_ms)?;
        }
        Ok(())
    }

    fn find_trigger_record_by_event(
        &mut self,
        trigger_id: &str,
        event_id: &str,
    ) -> Result<Option<TriggerEventRecord>, RuntimeStateError> {
        Ok(self
            .load_trigger_records_after_sequence(trigger_id, 0)?
            .into_iter()
            .find(|record| record.event_id == event_id))
    }

    pub fn write_trigger_record(
        &mut self,
        record: &TriggerEventRecord,
    ) -> Result<String, RuntimeStateError> {
        let sequence = i64::try_from(record.sequence).unwrap_or(i64::MAX);
        let payload_json = serde_json::to_string(&record.payload).map_err(|source| {
            RuntimeStateError::JsonEncode {
                field: "trigger_event_records.payload_json",
                source,
            }
        })?;

        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "INSERT INTO trigger_event_records (
                            schema_version, run_id, sequence, trigger_id, workflow_id, event_id,
                            checkpoint, source, accepted_at_ms, payload_json,
                            dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                        params![
                            record.schema_version,
                            record.run_id,
                            sequence,
                            record.trigger_id,
                            record.workflow_id,
                            record.event_id,
                            record.checkpoint,
                            record.source,
                            record.accepted_at_ms,
                            payload_json,
                            record.dedup_key,
                            record.dedup_expires_at_ms,
                            record.cooldown_key,
                            record.cooldown_expires_at_ms
                        ],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "insert sqlite trigger event record",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client
                    .execute(
                        "INSERT INTO trigger_event_records (
                            schema_version, run_id, sequence, trigger_id, workflow_id, event_id,
                            checkpoint, source, accepted_at_ms, payload_json,
                            dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
                         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
                        &[
                            &record.schema_version,
                            &record.run_id,
                            &sequence,
                            &record.trigger_id,
                            &record.workflow_id,
                            &record.event_id,
                            &record.checkpoint,
                            &record.source,
                            &record.accepted_at_ms,
                            &payload_json,
                            &record.dedup_key,
                            &record.dedup_expires_at_ms,
                            &record.cooldown_key,
                            &record.cooldown_expires_at_ms,
                        ],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "insert postgres trigger event record",
                        source,
                    })?;
            }
        }

        Ok(format!(
            "db://trigger_event_records/{}/{}",
            record.trigger_id, record.sequence
        ))
    }

    pub fn read_trigger_checkpoint(
        &mut self,
        trigger_id: &str,
    ) -> Result<Option<TriggerCheckpointRecord>, RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let row = connection
                    .query_row(
                        "SELECT schema_version, trigger_id, checkpoint, acked_at_ms
                         FROM trigger_checkpoints WHERE trigger_id = ?1",
                        params![trigger_id],
                        |row| {
                            Ok(TriggerCheckpointRecord {
                                schema_version: row.get(0)?,
                                trigger_id: row.get(1)?,
                                checkpoint: row.get(2)?,
                                acked_at_ms: row.get(3)?,
                            })
                        },
                    )
                    .optional()
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "query sqlite trigger checkpoint",
                        source,
                    })?;
                Ok(row)
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let row = client
                    .query_opt(
                        "SELECT schema_version, trigger_id, checkpoint, acked_at_ms
                         FROM trigger_checkpoints WHERE trigger_id = $1",
                        &[&trigger_id],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "query postgres trigger checkpoint",
                        source,
                    })?;
                Ok(row.map(|row| TriggerCheckpointRecord {
                    schema_version: row.get(0),
                    trigger_id: row.get(1),
                    checkpoint: row.get(2),
                    acked_at_ms: row.get(3),
                }))
            }
        }
    }

    pub fn write_trigger_checkpoint(
        &mut self,
        checkpoint: &TriggerCheckpointRecord,
    ) -> Result<(), RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "INSERT INTO trigger_checkpoints (schema_version, trigger_id, checkpoint, acked_at_ms)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(trigger_id)
                         DO UPDATE SET schema_version = excluded.schema_version,
                                       checkpoint = excluded.checkpoint,
                                       acked_at_ms = excluded.acked_at_ms",
                        params![
                            checkpoint.schema_version,
                            checkpoint.trigger_id,
                            checkpoint.checkpoint,
                            checkpoint.acked_at_ms
                        ],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "upsert sqlite trigger checkpoint",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client
                    .execute(
                        "INSERT INTO trigger_checkpoints (schema_version, trigger_id, checkpoint, acked_at_ms)
                         VALUES ($1, $2, $3, $4)
                         ON CONFLICT(trigger_id)
                         DO UPDATE SET schema_version = EXCLUDED.schema_version,
                                       checkpoint = EXCLUDED.checkpoint,
                                       acked_at_ms = EXCLUDED.acked_at_ms",
                        &[
                            &checkpoint.schema_version,
                            &checkpoint.trigger_id,
                            &checkpoint.checkpoint,
                            &checkpoint.acked_at_ms,
                        ],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "upsert postgres trigger checkpoint",
                        source,
                    })?;
            }
        }
        Ok(())
    }

    pub fn dedup_is_ready(
        &mut self,
        dedup_key: &str,
        now_ms: i64,
    ) -> Result<bool, RuntimeStateError> {
        self.token_is_ready("dedup_key", dedup_key, now_ms)
    }

    pub fn cooldown_is_ready(
        &mut self,
        cooldown_key: &str,
        now_ms: i64,
    ) -> Result<bool, RuntimeStateError> {
        self.token_is_ready("cooldown_key", cooldown_key, now_ms)
    }

    pub fn apply_history_retention(
        &mut self,
        policy: &RuntimeHistoryRetentionPolicy,
        now_ms: i64,
    ) -> Result<RuntimeHistoryArchiveStats, RuntimeStateError> {
        let mut stats = RuntimeHistoryArchiveStats::default();
        if let Some(retention_ms) = policy.run_retention_ms {
            let cutoff_ms = now_ms.saturating_sub(retention_ms);
            stats.archived_run_summaries = self.archive_run_summaries_before(cutoff_ms, now_ms)?;
        }
        if let Some(retention_ms) = policy.workflow_log_retention_ms {
            let cutoff_ms = now_ms.saturating_sub(retention_ms);
            stats.archived_workflow_logs = self.archive_workflow_logs_before(cutoff_ms, now_ms)?;
        }
        if let Some(retention_ms) = policy.trigger_event_retention_ms {
            let cutoff_ms = now_ms.saturating_sub(retention_ms);
            stats.archived_trigger_events =
                self.archive_trigger_events_before(cutoff_ms, now_ms)?;
        }
        Ok(stats)
    }

    pub fn archived_history_counts(
        &mut self,
    ) -> Result<RuntimeHistoryArchiveCounts, RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                Ok(RuntimeHistoryArchiveCounts {
                    run_summaries: sqlite_count(
                        connection,
                        path,
                        "archived_run_summaries",
                        "count sqlite archived run_summaries",
                    )?,
                    workflow_logs: sqlite_count(
                        connection,
                        path,
                        "archived_workflow_runtime_logs",
                        "count sqlite archived workflow logs",
                    )?,
                    trigger_events: sqlite_count(
                        connection,
                        path,
                        "archived_trigger_event_records",
                        "count sqlite archived trigger events",
                    )?,
                })
            }
            RuntimeStorageConnection::Postgres { client, .. } => Ok(RuntimeHistoryArchiveCounts {
                run_summaries: postgres_count(
                    client,
                    "archived_run_summaries",
                    "count postgres archived run_summaries",
                )?,
                workflow_logs: postgres_count(
                    client,
                    "archived_workflow_runtime_logs",
                    "count postgres archived workflow logs",
                )?,
                trigger_events: postgres_count(
                    client,
                    "archived_trigger_event_records",
                    "count postgres archived trigger events",
                )?,
            }),
        }
    }

    pub fn recover_runtime_state(
        &mut self,
        recovered_at_ms: i64,
    ) -> Result<usize, RuntimeStateError> {
        let incomplete_runs = match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let mut statement = connection
                    .prepare(
                        "SELECT run_id, workflow_id, started_at_ms FROM run_summaries
                         WHERE (status = 'pending' OR status = 'running') AND finished_at_ms IS NULL",
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "prepare sqlite incomplete runs query",
                        source,
                    })?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    })
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "query sqlite incomplete runs",
                        source,
                    })?;
                let mut runs = Vec::new();
                for row in rows {
                    runs.push(row.map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "decode sqlite incomplete run row",
                        source,
                    })?);
                }
                runs
            }
            RuntimeStorageConnection::Postgres { client, .. } => client
                .query(
                    "SELECT run_id, workflow_id, started_at_ms FROM run_summaries
                     WHERE (status = 'pending' OR status = 'running') AND finished_at_ms IS NULL",
                    &[],
                )
                .map_err(|source| RuntimeStateError::Postgres {
                    operation: "query postgres incomplete runs",
                    source,
                })?
                .into_iter()
                .map(|row| {
                    (
                        row.get::<_, String>(0),
                        row.get::<_, String>(1),
                        row.get::<_, i64>(2),
                    )
                })
                .collect::<Vec<_>>(),
        };

        let mut recovered_count = 0;
        for (run_id, workflow_id, started_at_ms) in incomplete_runs {
            self.append_workflow_log_entry(
                &run_id,
                "run_recovered_after_restart",
                "restart recovery marked an incomplete run as failed",
                recovered_at_ms,
            )?;
            self.write_run_summary(&RunRecordSummary {
                schema_version: String::from("1.0.0"),
                run_id,
                workflow_id,
                status: RunStatus::Failed,
                started_at_ms,
                finished_at_ms: Some(recovered_at_ms),
                owner_id: None,
                lease_generation: None,
            })?;
            recovered_count += 1;
        }

        Ok(recovered_count)
    }

    fn archive_run_summaries_before(
        &mut self,
        cutoff_ms: i64,
        archived_at_ms: i64,
    ) -> Result<u64, RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let transaction =
                    connection
                        .transaction()
                        .map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "start sqlite run archive transaction",
                            source,
                        })?;
                let inserted = transaction
                    .execute(
                        "INSERT INTO archived_run_summaries (
                            archived_at_ms, run_id, schema_version, workflow_id, status, started_at_ms, finished_at_ms
                         )
                         SELECT ?1, run_id, schema_version, workflow_id, status, started_at_ms, finished_at_ms
                         FROM run_summaries
                         WHERE finished_at_ms IS NOT NULL AND finished_at_ms < ?2",
                        params![archived_at_ms, cutoff_ms],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "archive sqlite run summaries",
                        source,
                    })?;
                transaction
                    .execute(
                        "DELETE FROM run_summaries
                         WHERE finished_at_ms IS NOT NULL AND finished_at_ms < ?1",
                        params![cutoff_ms],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "delete archived sqlite run summaries",
                        source,
                    })?;
                transaction
                    .commit()
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "commit sqlite run archive transaction",
                        source,
                    })?;
                Ok(u64::try_from(inserted).unwrap_or(0))
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let mut transaction =
                    client
                        .transaction()
                        .map_err(|source| RuntimeStateError::Postgres {
                            operation: "start postgres run archive transaction",
                            source,
                        })?;
                let inserted = transaction
                    .execute(
                        "INSERT INTO archived_run_summaries (
                            archived_at_ms, run_id, schema_version, workflow_id, status, started_at_ms, finished_at_ms
                         )
                         SELECT $1, run_id, schema_version, workflow_id, status, started_at_ms, finished_at_ms
                         FROM run_summaries
                         WHERE finished_at_ms IS NOT NULL AND finished_at_ms < $2",
                        &[&archived_at_ms, &cutoff_ms],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "archive postgres run summaries",
                        source,
                    })?;
                transaction
                    .execute(
                        "DELETE FROM run_summaries
                         WHERE finished_at_ms IS NOT NULL AND finished_at_ms < $1",
                        &[&cutoff_ms],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "delete archived postgres run summaries",
                        source,
                    })?;
                transaction
                    .commit()
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "commit postgres run archive transaction",
                        source,
                    })?;
                Ok(inserted)
            }
        }
    }

    fn archive_workflow_logs_before(
        &mut self,
        cutoff_ms: i64,
        archived_at_ms: i64,
    ) -> Result<u64, RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let transaction =
                    connection
                        .transaction()
                        .map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "start sqlite workflow log archive transaction",
                            source,
                        })?;
                let inserted = transaction
                    .execute(
                        "INSERT INTO archived_workflow_runtime_logs (
                        archived_at_ms, run_id, sequence, event, message, occurred_at_ms
                     )
                     SELECT ?1, run_id, sequence, event, message, occurred_at_ms
                     FROM workflow_runtime_logs
                     WHERE occurred_at_ms < ?2",
                        params![archived_at_ms, cutoff_ms],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "archive sqlite workflow logs",
                        source,
                    })?;
                transaction
                    .execute(
                        "DELETE FROM workflow_runtime_logs WHERE occurred_at_ms < ?1",
                        params![cutoff_ms],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "delete archived sqlite workflow logs",
                        source,
                    })?;
                transaction
                    .commit()
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "commit sqlite workflow log archive transaction",
                        source,
                    })?;
                Ok(u64::try_from(inserted).unwrap_or(0))
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let mut transaction =
                    client
                        .transaction()
                        .map_err(|source| RuntimeStateError::Postgres {
                            operation: "start postgres workflow log archive transaction",
                            source,
                        })?;
                let inserted = transaction
                    .execute(
                        "INSERT INTO archived_workflow_runtime_logs (
                        archived_at_ms, run_id, sequence, event, message, occurred_at_ms
                     )
                     SELECT $1, run_id, sequence, event, message, occurred_at_ms
                     FROM workflow_runtime_logs
                     WHERE occurred_at_ms < $2",
                        &[&archived_at_ms, &cutoff_ms],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "archive postgres workflow logs",
                        source,
                    })?;
                transaction
                    .execute(
                        "DELETE FROM workflow_runtime_logs WHERE occurred_at_ms < $1",
                        &[&cutoff_ms],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "delete archived postgres workflow logs",
                        source,
                    })?;
                transaction
                    .commit()
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "commit postgres workflow log archive transaction",
                        source,
                    })?;
                Ok(inserted)
            }
        }
    }

    fn archive_trigger_events_before(
        &mut self,
        cutoff_ms: i64,
        archived_at_ms: i64,
    ) -> Result<u64, RuntimeStateError> {
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let transaction =
                    connection
                        .transaction()
                        .map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "start sqlite trigger archive transaction",
                            source,
                        })?;
                let inserted = transaction.execute(
                    "INSERT INTO archived_trigger_event_records (
                        archived_at_ms, trigger_id, sequence, schema_version, run_id, workflow_id, event_id,
                        checkpoint, source, accepted_at_ms, payload_json,
                        dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
                     )
                     SELECT ?1, trigger_id, sequence, schema_version, run_id, workflow_id, event_id,
                            checkpoint, source, accepted_at_ms, payload_json,
                            dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
                     FROM trigger_event_records
                     WHERE accepted_at_ms < ?2
                       AND COALESCE(dedup_expires_at_ms, ?1) <= ?1
                       AND COALESCE(cooldown_expires_at_ms, ?1) <= ?1",
                    params![archived_at_ms, cutoff_ms],
                ).map_err(|source| RuntimeStateError::Sqlite {
                    path: path.clone(),
                    operation: "archive sqlite trigger events",
                    source,
                })?;
                transaction
                    .execute(
                        "DELETE FROM trigger_event_records
                         WHERE accepted_at_ms < ?2
                           AND COALESCE(dedup_expires_at_ms, ?1) <= ?1
                           AND COALESCE(cooldown_expires_at_ms, ?1) <= ?1",
                        params![archived_at_ms, cutoff_ms],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "delete archived sqlite trigger events",
                        source,
                    })?;
                transaction
                    .commit()
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "commit sqlite trigger archive transaction",
                        source,
                    })?;
                Ok(u64::try_from(inserted).unwrap_or(0))
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let mut transaction =
                    client
                        .transaction()
                        .map_err(|source| RuntimeStateError::Postgres {
                            operation: "start postgres trigger archive transaction",
                            source,
                        })?;
                let inserted = transaction.execute(
                    "INSERT INTO archived_trigger_event_records (
                        archived_at_ms, trigger_id, sequence, schema_version, run_id, workflow_id, event_id,
                        checkpoint, source, accepted_at_ms, payload_json,
                        dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
                     )
                     SELECT $1, trigger_id, sequence, schema_version, run_id, workflow_id, event_id,
                            checkpoint, source, accepted_at_ms, payload_json,
                            dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
                     FROM trigger_event_records
                     WHERE accepted_at_ms < $2
                       AND COALESCE(dedup_expires_at_ms, $1) <= $1
                       AND COALESCE(cooldown_expires_at_ms, $1) <= $1",
                    &[&archived_at_ms, &cutoff_ms],
                ).map_err(|source| RuntimeStateError::Postgres {
                    operation: "archive postgres trigger events",
                    source,
                })?;
                transaction
                    .execute(
                        "DELETE FROM trigger_event_records
                         WHERE accepted_at_ms < $2
                           AND COALESCE(dedup_expires_at_ms, $1) <= $1
                           AND COALESCE(cooldown_expires_at_ms, $1) <= $1",
                        &[&archived_at_ms, &cutoff_ms],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "delete archived postgres trigger events",
                        source,
                    })?;
                transaction
                    .commit()
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "commit postgres trigger archive transaction",
                        source,
                    })?;
                Ok(inserted)
            }
        }
    }

    fn token_is_ready(
        &mut self,
        key_column: &'static str,
        key: &str,
        now_ms: i64,
    ) -> Result<bool, RuntimeStateError> {
        let query_sqlite = format!(
            "SELECT 1 FROM trigger_event_records WHERE {key_column} = ?1 AND {} > ?2 LIMIT 1",
            key_column.replace("_key", "_expires_at_ms")
        );
        let query_postgres = format!(
            "SELECT 1 FROM trigger_event_records WHERE {key_column} = $1 AND {} > $2 LIMIT 1",
            key_column.replace("_key", "_expires_at_ms")
        );

        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let exists = connection
                    .query_row(&query_sqlite, params![key, now_ms], |_| Ok(1_i64))
                    .optional()
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "query sqlite token readiness",
                        source,
                    })?
                    .is_some();
                Ok(!exists)
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let exists = client
                    .query_opt(&query_postgres, &[&key, &now_ms])
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "query postgres token readiness",
                        source,
                    })?
                    .is_some();
                Ok(!exists)
            }
        }
    }

    fn next_workflow_log_sequence(&mut self, run_id: &str) -> Result<u64, RuntimeStateError> {
        let next = match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => connection
                .query_row(
                    "SELECT COALESCE(MAX(sequence), 0) + 1 FROM workflow_runtime_logs WHERE run_id = ?1",
                    params![run_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|source| RuntimeStateError::Sqlite {
                    path: path.clone(),
                    operation: "query sqlite next workflow log sequence",
                    source,
                })?,
            RuntimeStorageConnection::Postgres { client, .. } => client
                .query_one(
                    "SELECT COALESCE(MAX(sequence), 0) + 1 FROM workflow_runtime_logs WHERE run_id = $1",
                    &[&run_id],
                )
                .map_err(|source| RuntimeStateError::Postgres {
                    operation: "query postgres next workflow log sequence",
                    source,
                })?
                .get::<_, i64>(0),
        };
        Ok(u64::try_from(next).unwrap_or(1))
    }

    fn run_migrations(&mut self, now_ms: i64) -> Result<(), RuntimeStateError> {
        let create_schema_sql = format!(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version BIGINT PRIMARY KEY,
                applied_at_ms BIGINT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS serve_leases (
                 lease_key TEXT PRIMARY KEY,
                 owner_id TEXT NOT NULL,
                 acquired_at_ms BIGINT NOT NULL,
                 expires_at_ms BIGINT NOT NULL,
                 generation BIGINT NOT NULL DEFAULT 0
              );

             CREATE TABLE IF NOT EXISTS daemon_sessions (
                 lease_key TEXT PRIMARY KEY,
                 owner_id TEXT NOT NULL,
                 pid BIGINT NULL,
                 started_at_ms BIGINT NOT NULL,
                 last_heartbeat_at_ms BIGINT NOT NULL,
                 lease_expires_at_ms BIGINT NOT NULL,
                 last_reload_at_ms BIGINT NULL,
                 stop_requested_at_ms BIGINT NULL,
                 stopped_at_ms BIGINT NULL,
                 last_error_code TEXT NULL,
                 last_error_message TEXT NULL
              );

             CREATE TABLE IF NOT EXISTS run_summaries (
                run_id TEXT PRIMARY KEY,
                schema_version TEXT NOT NULL,
                workflow_id TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at_ms BIGINT NOT NULL,
                finished_at_ms BIGINT NULL,
                owner_id TEXT NULL,
                lease_generation BIGINT NULL
             );

             CREATE TABLE IF NOT EXISTS workflow_runtime_logs (
                run_id TEXT NOT NULL,
                sequence BIGINT NOT NULL,
                event TEXT NOT NULL,
                message TEXT NOT NULL,
                occurred_at_ms BIGINT NOT NULL,
                PRIMARY KEY (run_id, sequence)
             );

             CREATE TABLE IF NOT EXISTS trigger_event_records (
                trigger_id TEXT NOT NULL,
                sequence BIGINT NOT NULL,
                schema_version TEXT NOT NULL,
                run_id TEXT NOT NULL,
                workflow_id TEXT NOT NULL,
                event_id TEXT NOT NULL,
                checkpoint TEXT NULL,
                source TEXT NOT NULL,
                accepted_at_ms BIGINT NOT NULL,
                payload_json TEXT NOT NULL,
                dedup_key TEXT NULL,
                dedup_expires_at_ms BIGINT NULL,
                cooldown_key TEXT NULL,
                cooldown_expires_at_ms BIGINT NULL,
                PRIMARY KEY (trigger_id, sequence),
                UNIQUE (trigger_id, event_id)
             );

             CREATE TABLE IF NOT EXISTS trigger_checkpoints (
                trigger_id TEXT PRIMARY KEY,
                schema_version TEXT NOT NULL,
                checkpoint TEXT NOT NULL,
                acked_at_ms BIGINT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS trigger_snapshots (
                 trigger_id TEXT PRIMARY KEY,
                 schema_version TEXT NOT NULL,
                 last_event_id TEXT NULL,
                 last_accepted_at_ms BIGINT NULL,
                 last_sequence BIGINT NOT NULL,
                 accepted_event_ids_json TEXT NOT NULL,
                 dedup_tokens_json TEXT NOT NULL,
                 cooldown_tokens_json TEXT NOT NULL
              );

             CREATE TABLE IF NOT EXISTS ingress_inbox_records (
                 inbox_id TEXT PRIMARY KEY,
                 schema_version TEXT NOT NULL,
                 trigger_id TEXT NOT NULL,
                 workflow_id TEXT NOT NULL,
                 transport_kind TEXT NOT NULL,
                 ingress_event_id TEXT NOT NULL,
                 source TEXT NOT NULL,
                 route_path TEXT NOT NULL,
                 http_method TEXT NULL,
                 received_at_ms BIGINT NOT NULL,
                 payload_json TEXT NOT NULL,
                 headers_json TEXT NOT NULL,
                 remote_addr TEXT NULL,
                 processed_at_ms BIGINT NULL,
                 last_error TEXT NULL,
                 UNIQUE (trigger_id, ingress_event_id)
               );

             CREATE TABLE IF NOT EXISTS staged_trigger_event_records (
                 staging_id TEXT PRIMARY KEY,
                 schema_version TEXT NOT NULL,
                 trigger_id TEXT NOT NULL,
                 workflow_id TEXT NOT NULL,
                 event_id TEXT NOT NULL,
                 source TEXT NOT NULL,
                 occurred_at_ms BIGINT NOT NULL,
                 staged_at_ms BIGINT NOT NULL,
                 checkpoint TEXT NULL,
                 payload_json TEXT NOT NULL,
                 dedup_key TEXT NULL,
                 dedup_window_ms BIGINT NULL,
                 cooldown_key TEXT NULL,
                 cooldown_ms BIGINT NULL,
                 accepted_at_ms BIGINT NULL,
                 last_error TEXT NULL,
                 UNIQUE (trigger_id, event_id)
              );

             CREATE TABLE IF NOT EXISTS archived_run_summaries (
                archived_at_ms BIGINT NOT NULL,
                run_id TEXT NOT NULL,
                schema_version TEXT NOT NULL,
                workflow_id TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at_ms BIGINT NOT NULL,
                finished_at_ms BIGINT NULL,
                PRIMARY KEY (archived_at_ms, run_id)
             );

             CREATE TABLE IF NOT EXISTS archived_workflow_runtime_logs (
                archived_at_ms BIGINT NOT NULL,
                run_id TEXT NOT NULL,
                sequence BIGINT NOT NULL,
                event TEXT NOT NULL,
                message TEXT NOT NULL,
                occurred_at_ms BIGINT NOT NULL,
                PRIMARY KEY (archived_at_ms, run_id, sequence)
             );

             CREATE TABLE IF NOT EXISTS archived_trigger_event_records (
                archived_at_ms BIGINT NOT NULL,
                trigger_id TEXT NOT NULL,
                sequence BIGINT NOT NULL,
                schema_version TEXT NOT NULL,
                run_id TEXT NOT NULL,
                workflow_id TEXT NOT NULL,
                event_id TEXT NOT NULL,
                checkpoint TEXT NULL,
                source TEXT NOT NULL,
                accepted_at_ms BIGINT NOT NULL,
                payload_json TEXT NOT NULL,
                dedup_key TEXT NULL,
                dedup_expires_at_ms BIGINT NULL,
                cooldown_key TEXT NULL,
                cooldown_expires_at_ms BIGINT NULL,
                PRIMARY KEY (archived_at_ms, trigger_id, sequence)
             );

             CREATE INDEX IF NOT EXISTS idx_run_summaries_started_at
             ON run_summaries (started_at_ms DESC, run_id DESC);

             CREATE INDEX IF NOT EXISTS idx_workflow_runtime_logs_run_sequence
             ON workflow_runtime_logs (run_id, sequence DESC);

             CREATE INDEX IF NOT EXISTS idx_workflow_runtime_logs_occurred_at
             ON workflow_runtime_logs (occurred_at_ms DESC, run_id DESC, sequence DESC);

             CREATE INDEX IF NOT EXISTS idx_trigger_event_records_accepted_at
             ON trigger_event_records (accepted_at_ms DESC, trigger_id DESC, sequence DESC);

             CREATE INDEX IF NOT EXISTS idx_trigger_event_records_trigger_sequence
             ON trigger_event_records (trigger_id, sequence DESC);

             CREATE INDEX IF NOT EXISTS idx_trigger_event_records_run_id
             ON trigger_event_records (run_id);

             CREATE INDEX IF NOT EXISTS idx_ingress_inbox_records_trigger_received
             ON ingress_inbox_records (trigger_id, processed_at_ms, received_at_ms ASC, inbox_id ASC);

             CREATE INDEX IF NOT EXISTS idx_staged_trigger_event_records_pending
             ON staged_trigger_event_records (trigger_id, accepted_at_ms, staged_at_ms ASC, staging_id ASC);

             CREATE INDEX IF NOT EXISTS idx_staged_trigger_event_records_reconcile
             ON staged_trigger_event_records (accepted_at_ms, staged_at_ms ASC, trigger_id ASC, staging_id ASC);

             CREATE INDEX IF NOT EXISTS idx_daemon_sessions_owner
             ON daemon_sessions (owner_id, stopped_at_ms, lease_expires_at_ms DESC);

             CREATE INDEX IF NOT EXISTS idx_archived_run_summaries_started_at
             ON archived_run_summaries (started_at_ms DESC, run_id DESC);

             CREATE INDEX IF NOT EXISTS idx_archived_workflow_runtime_logs_occurred_at
             ON archived_workflow_runtime_logs (occurred_at_ms DESC, run_id DESC, sequence DESC);

             CREATE INDEX IF NOT EXISTS idx_archived_trigger_event_records_accepted_at
             ON archived_trigger_event_records (accepted_at_ms DESC, trigger_id DESC, sequence DESC);"
        );

        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute_batch(&create_schema_sql)
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "apply sqlite runtime schema migrations",
                        source,
                    })?;
                let has_generation = connection
                    .query_row(
                        "SELECT COUNT(*) FROM pragma_table_info('serve_leases') WHERE name = 'generation'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "inspect sqlite serve lease generation migration",
                        source,
                    })?
                    > 0;
                if !has_generation {
                    connection
                        .execute("ALTER TABLE serve_leases ADD COLUMN generation BIGINT NOT NULL DEFAULT 0", [])
                        .map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "apply sqlite serve lease generation migration",
                            source,
                        })?;
                }
                for column in ["owner_id TEXT", "lease_generation BIGINT"] {
                    let column_name = column.split_once(' ').map(|(name, _)| name).unwrap_or_default();
                    let exists = connection
                        .query_row(
                            "SELECT COUNT(*) FROM pragma_table_info('run_summaries') WHERE name = ?1",
                            params![column_name],
                            |row| row.get::<_, i64>(0),
                        )
                        .map_err(|source| RuntimeStateError::Sqlite {
                            path: path.clone(),
                            operation: "inspect sqlite run summary fence migration",
                            source,
                        })?
                        > 0;
                    if !exists {
                        connection
                            .execute(&format!("ALTER TABLE run_summaries ADD COLUMN {column}"), [])
                            .map_err(|source| RuntimeStateError::Sqlite {
                                path: path.clone(),
                                operation: "apply sqlite run summary fence migration",
                                source,
                            })?;
                    }
                }
                connection
                    .execute(
                        "INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
                        params![5_i64, now_ms],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "record sqlite schema migration",
                        source,
                    })?;
                connection
                    .execute(
                        "INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
                        params![6_i64, now_ms],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "record sqlite serve lease generation migration",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client.batch_execute(&create_schema_sql).map_err(|source| {
                    RuntimeStateError::Postgres {
                        operation: "apply postgres runtime schema migrations",
                        source,
                    }
                })?;
                client
                    .batch_execute(
                        "ALTER TABLE serve_leases ADD COLUMN IF NOT EXISTS generation BIGINT NOT NULL DEFAULT 0;
                         ALTER TABLE run_summaries ADD COLUMN IF NOT EXISTS owner_id TEXT NULL;
                         ALTER TABLE run_summaries ADD COLUMN IF NOT EXISTS lease_generation BIGINT NULL;",
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "apply postgres runtime fence migration",
                        source,
                    })?;
                client
                    .execute(
                        "INSERT INTO schema_migrations (version, applied_at_ms)
                         VALUES ($1, $2)
                         ON CONFLICT(version) DO NOTHING",
                        &[&5_i64, &now_ms],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "record postgres schema migration",
                        source,
                    })?;
                client
                    .execute(
                        "INSERT INTO schema_migrations (version, applied_at_ms)
                         VALUES ($1, $2)
                         ON CONFLICT(version) DO NOTHING",
                        &[&6_i64, &now_ms],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "record postgres serve lease generation migration",
                        source,
                    })?;
            }
        }

        Ok(())
    }
}

impl Display for RuntimeStateError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io {
                path,
                operation,
                source,
            } => write!(f, "failed to {operation} at {}: {source}", path.display()),
            Self::Sqlite {
                path,
                operation,
                source,
            } => write!(f, "failed to {operation} at {}: {source}", path.display()),
            Self::Postgres { operation, source } => write!(f, "failed to {operation}: {source}"),
            Self::JsonEncode { field, source } => {
                write!(f, "failed to encode JSON for {field}: {source}")
            }
            Self::JsonDecode { field, source } => {
                write!(f, "failed to decode JSON for {field}: {source}")
            }
            Self::LeaseFenceLost { run_id } => {
                write!(f, "lease fence no longer owns run {run_id}")
            }
        }
    }
}

impl Error for RuntimeStateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Sqlite { source, .. } => Some(source),
            Self::Postgres { source, .. } => Some(source),
            Self::JsonEncode { source, .. } => Some(source),
            Self::JsonDecode { source, .. } => Some(source),
            Self::LeaseFenceLost { .. } => None,
        }
    }
}

fn sqlite_count(
    connection: &Connection,
    path: &PathBuf,
    table: &'static str,
    operation: &'static str,
) -> Result<u64, RuntimeStateError> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let count = connection
        .query_row(&sql, [], |row| row.get::<_, i64>(0))
        .map_err(|source| RuntimeStateError::Sqlite {
            path: path.clone(),
            operation,
            source,
        })?;
    Ok(u64::try_from(count).unwrap_or(0))
}

fn postgres_count(
    client: &mut Client,
    table: &'static str,
    operation: &'static str,
) -> Result<u64, RuntimeStateError> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let count = client
        .query_one(&sql, &[])
        .map_err(|source| RuntimeStateError::Postgres { operation, source })?
        .get::<_, i64>(0);
    Ok(u64::try_from(count).unwrap_or(0))
}

fn serve_snapshot_from_row(row: Option<(String, i64)>, now_ms: i64) -> ServeLeaseSnapshot {
    match row {
        None => ServeLeaseSnapshot {
            state: ServeLeaseState::Idle,
            owner_id: None,
            expires_at_ms: None,
        },
        Some((owner_id, expires_at_ms)) if expires_at_ms > now_ms => ServeLeaseSnapshot {
            state: ServeLeaseState::Active,
            owner_id: Some(owner_id),
            expires_at_ms: Some(expires_at_ms),
        },
        Some((owner_id, expires_at_ms)) => ServeLeaseSnapshot {
            state: ServeLeaseState::Stale,
            owner_id: Some(owner_id),
            expires_at_ms: Some(expires_at_ms),
        },
    }
}

fn build_runtime_daemon_status(
    daemon_row: Option<DaemonSessionRow>,
    lease_row: Option<(String, i64)>,
    now_ms: i64,
) -> RuntimeDaemonStatus {
    let Some(daemon_row) = daemon_row else {
        let lease_snapshot = serve_snapshot_from_row(lease_row, now_ms);
        return RuntimeDaemonStatus {
            state: lease_snapshot.state,
            owner_id: lease_snapshot.owner_id,
            pid: None,
            started_at_ms: None,
            last_heartbeat_at_ms: None,
            lease_expires_at_ms: lease_snapshot.expires_at_ms,
            last_reload_at_ms: None,
            stop_requested_at_ms: None,
            stopped_at_ms: None,
            last_error_code: None,
            last_error_message: None,
        };
    };

    if daemon_row.stopped_at_ms.is_some() {
        return RuntimeDaemonStatus {
            state: ServeLeaseState::Idle,
            owner_id: None,
            pid: daemon_row.pid,
            started_at_ms: Some(daemon_row.started_at_ms),
            last_heartbeat_at_ms: Some(daemon_row.last_heartbeat_at_ms),
            lease_expires_at_ms: None,
            last_reload_at_ms: daemon_row.last_reload_at_ms,
            stop_requested_at_ms: daemon_row.stop_requested_at_ms,
            stopped_at_ms: daemon_row.stopped_at_ms,
            last_error_code: daemon_row.last_error_code,
            last_error_message: daemon_row.last_error_message,
        };
    }

    let lease_expires_at_ms = lease_row
        .as_ref()
        .map(|(_, expires_at_ms)| *expires_at_ms)
        .unwrap_or(daemon_row.lease_expires_at_ms);
    let state = if lease_expires_at_ms > now_ms {
        ServeLeaseState::Active
    } else {
        ServeLeaseState::Stale
    };

    RuntimeDaemonStatus {
        state,
        owner_id: Some(daemon_row.owner_id),
        pid: daemon_row.pid,
        started_at_ms: Some(daemon_row.started_at_ms),
        last_heartbeat_at_ms: Some(daemon_row.last_heartbeat_at_ms),
        lease_expires_at_ms: Some(lease_expires_at_ms),
        last_reload_at_ms: daemon_row.last_reload_at_ms,
        stop_requested_at_ms: daemon_row.stop_requested_at_ms,
        stopped_at_ms: daemon_row.stopped_at_ms,
        last_error_code: daemon_row.last_error_code,
        last_error_message: daemon_row.last_error_message,
    }
}

fn parse_run_status(value: &str) -> RunStatus {
    match value {
        "pending" => RunStatus::Pending,
        "running" => RunStatus::Running,
        "succeeded" => RunStatus::Succeeded,
        "failed" => RunStatus::Failed,
        _ => RunStatus::Failed,
    }
}

fn render_run_status(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Pending => "pending",
        RunStatus::Running => "running",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed => "failed",
    }
}

fn decode_snapshot(
    schema_version: String,
    trigger_id: String,
    last_event_id: Option<String>,
    last_accepted_at_ms: Option<i64>,
    last_sequence: i64,
    accepted_json: &str,
    dedup_json: &str,
    cooldown_json: &str,
) -> Result<TriggerSnapshotRecord, RuntimeStateError> {
    Ok(TriggerSnapshotRecord {
        schema_version,
        trigger_id,
        last_event_id,
        last_accepted_at_ms,
        last_sequence: u64::try_from(last_sequence).unwrap_or(0),
        accepted_event_ids: serde_json::from_str(accepted_json).map_err(|source| {
            RuntimeStateError::JsonDecode {
                field: "trigger_snapshots.accepted_event_ids_json",
                source,
            }
        })?,
        dedup_tokens: serde_json::from_str(dedup_json).map_err(|source| {
            RuntimeStateError::JsonDecode {
                field: "trigger_snapshots.dedup_tokens_json",
                source,
            }
        })?,
        cooldown_tokens: serde_json::from_str(cooldown_json).map_err(|source| {
            RuntimeStateError::JsonDecode {
                field: "trigger_snapshots.cooldown_tokens_json",
                source,
            }
        })?,
    })
}

type TriggerRecordRow = (
    String,
    String,
    i64,
    String,
    String,
    String,
    Option<String>,
    String,
    i64,
    String,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<i64>,
);

fn decode_trigger_record_row(
    row: TriggerRecordRow,
) -> Result<TriggerEventRecord, RuntimeStateError> {
    let (
        schema_version,
        run_id,
        sequence,
        trigger_id,
        workflow_id,
        event_id,
        checkpoint,
        source,
        accepted_at_ms,
        payload_json,
        dedup_key,
        dedup_expires_at_ms,
        cooldown_key,
        cooldown_expires_at_ms,
    ) = row;
    Ok(TriggerEventRecord {
        schema_version,
        run_id,
        sequence: u64::try_from(sequence).unwrap_or(0),
        trigger_id,
        workflow_id,
        event_id,
        checkpoint,
        source,
        accepted_at_ms,
        payload: serde_json::from_str(&payload_json).map_err(|source| {
            RuntimeStateError::JsonDecode {
                field: "trigger_event_records.payload_json",
                source,
            }
        })?,
        dedup_key,
        dedup_expires_at_ms,
        cooldown_key,
        cooldown_expires_at_ms,
    })
}

fn decode_ingress_inbox_record_row(
    row: (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        i64,
        String,
        String,
        Option<String>,
        Option<i64>,
        Option<String>,
    ),
) -> Result<IngressInboxRecord, RuntimeStateError> {
    let payload =
        serde_json::from_str(&row.10).map_err(|source| RuntimeStateError::JsonDecode {
            field: "ingress_inbox_records.payload_json",
            source,
        })?;
    let headers =
        serde_json::from_str(&row.11).map_err(|source| RuntimeStateError::JsonDecode {
            field: "ingress_inbox_records.headers_json",
            source,
        })?;
    Ok(IngressInboxRecord {
        inbox_id: row.0,
        schema_version: row.1,
        trigger_id: row.2,
        workflow_id: row.3,
        transport_kind: row.4,
        ingress_event_id: row.5,
        source: row.6,
        route_path: row.7,
        http_method: row.8,
        received_at_ms: row.9,
        payload,
        headers,
        remote_addr: row.12,
        processed_at_ms: row.13,
        last_error: row.14,
    })
}

type StagedTriggerEventRecordRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    Option<String>,
    String,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
);

fn decode_staged_trigger_event_record_row(
    row: StagedTriggerEventRecordRow,
) -> Result<StagedTriggerEventRecord, RuntimeStateError> {
    let payload = serde_json::from_str(&row.9).map_err(|source| RuntimeStateError::JsonDecode {
        field: "staged_trigger_event_records.payload_json",
        source,
    })?;
    Ok(StagedTriggerEventRecord {
        staging_id: row.0,
        schema_version: row.1,
        trigger_id: row.2,
        workflow_id: row.3,
        event_id: row.4,
        source: row.5,
        occurred_at_ms: row.6,
        staged_at_ms: row.7,
        checkpoint: row.8,
        payload,
        dedup_key: row.10,
        dedup_window_ms: row.11,
        cooldown_key: row.12,
        cooldown_ms: row.13,
        accepted_at_ms: row.14,
        last_error: row.15,
    })
}
