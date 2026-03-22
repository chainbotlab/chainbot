//! [INPUT]
//! Root storage configuration and runtime run/trigger state mutations for local SQLite or PostgreSQL backends.
//!
//! [OUTPUT]
//! Persists and reads authoritative run and trigger history from database tables used by CLI and trigger runtime paths.
//!
//! [ROLE]
//! Defines DB-primary runtime state for ChainBot main execution commands.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use postgres::{Client, NoTls};
use rusqlite::{params, Connection, OptionalExtension};

use crate::config::{RuntimeStorageBackend, RuntimeStorageConfig};
use crate::state::{
    LeaseAcquireResult, RunRecordSummary, RunStatus, ServeLeaseSnapshot, ServeLeaseState,
    TriggerCheckpointRecord, TriggerEventRecord, TriggerSnapshotRecord, SERVE_OWNER_ID_PREFIX,
};

const SERVE_LEASE_KEY: &str = "serve";

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
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                let current = connection
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
                Ok(serve_snapshot_from_row(current, now_ms))
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                let current = client
                    .query_opt(
                        "SELECT owner_id, expires_at_ms FROM serve_leases WHERE lease_key = $1",
                        &[&SERVE_LEASE_KEY],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "inspect postgres serve lease",
                        source,
                    })?
                    .map(|row| (row.get::<_, String>(0), row.get::<_, i64>(1)));
                Ok(serve_snapshot_from_row(current, now_ms))
            }
        }
    }

    pub fn try_acquire_serve_lease(
        &mut self,
        owner_id: &str,
        now_ms: i64,
        lease_ttl_ms: i64,
    ) -> Result<LeaseAcquireResult, RuntimeStateError> {
        let expires_at_ms = now_ms.saturating_add(lease_ttl_ms.max(1));
        let current = self.inspect_serve_lease(now_ms)?;

        if current.state == ServeLeaseState::Active
            && current.owner_id.as_deref() != Some(owner_id)
            && current.expires_at_ms.is_some()
        {
            return Ok(LeaseAcquireResult::Rejected {
                current_owner: current.owner_id.unwrap_or_default(),
                expires_at_ms: current.expires_at_ms.unwrap_or(now_ms),
            });
        }

        let result = if current.owner_id.as_deref() == Some(owner_id)
            && current.state == ServeLeaseState::Active
        {
            LeaseAcquireResult::Renewed
        } else {
            LeaseAcquireResult::Acquired
        };

        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "INSERT INTO serve_leases (lease_key, owner_id, acquired_at_ms, expires_at_ms)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(lease_key)
                         DO UPDATE SET owner_id = excluded.owner_id,
                                       acquired_at_ms = excluded.acquired_at_ms,
                                       expires_at_ms = excluded.expires_at_ms",
                        params![SERVE_LEASE_KEY, owner_id, now_ms, expires_at_ms],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "upsert sqlite serve lease",
                        source,
                    })?;
            }
            RuntimeStorageConnection::Postgres { client, .. } => {
                client
                    .execute(
                        "INSERT INTO serve_leases (lease_key, owner_id, acquired_at_ms, expires_at_ms)
                         VALUES ($1, $2, $3, $4)
                         ON CONFLICT(lease_key)
                         DO UPDATE SET owner_id = EXCLUDED.owner_id,
                                       acquired_at_ms = EXCLUDED.acquired_at_ms,
                                       expires_at_ms = EXCLUDED.expires_at_ms",
                        &[&SERVE_LEASE_KEY, &owner_id, &now_ms, &expires_at_ms],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "upsert postgres serve lease",
                        source,
                    })?;
            }
        }

        Ok(result)
    }

    pub fn release_serve_lease(&mut self, owner_id: &str) -> Result<bool, RuntimeStateError> {
        let rows = match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => connection
                .execute(
                    "DELETE FROM serve_leases WHERE lease_key = ?1 AND owner_id = ?2",
                    params![SERVE_LEASE_KEY, owner_id],
                )
                .map_err(|source| RuntimeStateError::Sqlite {
                    path: path.clone(),
                    operation: "release sqlite serve lease",
                    source,
                })?,
            RuntimeStorageConnection::Postgres { client, .. } => client
                .execute(
                    "DELETE FROM serve_leases WHERE lease_key = $1 AND owner_id = $2",
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
                        "SELECT schema_version, run_id, workflow_id, status, started_at_ms, finished_at_ms
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
                        "SELECT schema_version, run_id, workflow_id, status, started_at_ms, finished_at_ms
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

    pub fn write_run_summary(
        &mut self,
        summary: &RunRecordSummary,
    ) -> Result<(), RuntimeStateError> {
        let status = render_run_status(summary.status);
        match &mut self.connection {
            RuntimeStorageConnection::Sqlite { path, connection } => {
                connection
                    .execute(
                        "INSERT INTO run_summaries (schema_version, run_id, workflow_id, status, started_at_ms, finished_at_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                         ON CONFLICT(run_id)
                         DO UPDATE SET schema_version = excluded.schema_version,
                                       workflow_id = excluded.workflow_id,
                                       status = excluded.status,
                                       started_at_ms = excluded.started_at_ms,
                                       finished_at_ms = excluded.finished_at_ms",
                        params![
                            summary.schema_version,
                            summary.run_id,
                            summary.workflow_id,
                            status,
                            summary.started_at_ms,
                            summary.finished_at_ms
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
                        "INSERT INTO run_summaries (schema_version, run_id, workflow_id, status, started_at_ms, finished_at_ms)
                         VALUES ($1, $2, $3, $4, $5, $6)
                         ON CONFLICT(run_id)
                         DO UPDATE SET schema_version = EXCLUDED.schema_version,
                                       workflow_id = EXCLUDED.workflow_id,
                                       status = EXCLUDED.status,
                                       started_at_ms = EXCLUDED.started_at_ms,
                                       finished_at_ms = EXCLUDED.finished_at_ms",
                        &[
                            &summary.schema_version,
                            &summary.run_id,
                            &summary.workflow_id,
                            &status,
                            &summary.started_at_ms,
                            &summary.finished_at_ms,
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
            })?;
            recovered_count += 1;
        }

        Ok(recovered_count)
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
                expires_at_ms BIGINT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS run_summaries (
                run_id TEXT PRIMARY KEY,
                schema_version TEXT NOT NULL,
                workflow_id TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at_ms BIGINT NOT NULL,
                finished_at_ms BIGINT NULL
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
             );"
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
                connection
                    .execute(
                        "INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
                        params![1_i64, now_ms],
                    )
                    .map_err(|source| RuntimeStateError::Sqlite {
                        path: path.clone(),
                        operation: "record sqlite schema migration",
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
                    .execute(
                        "INSERT INTO schema_migrations (version, applied_at_ms)
                         VALUES ($1, $2)
                         ON CONFLICT(version) DO NOTHING",
                        &[&1_i64, &now_ms],
                    )
                    .map_err(|source| RuntimeStateError::Postgres {
                        operation: "record postgres schema migration",
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
        }
    }
}

fn serve_snapshot_from_row(row: Option<(String, i64)>, now_ms: i64) -> ServeLeaseSnapshot {
    match row {
        None => ServeLeaseSnapshot {
            state: ServeLeaseState::Idle,
            owner_id: None,
            expires_at_ms: None,
        },
        Some((owner_id, expires_at_ms))
            if expires_at_ms > now_ms && serve_owner_is_active(&owner_id) =>
        {
            ServeLeaseSnapshot {
                state: ServeLeaseState::Active,
                owner_id: Some(owner_id),
                expires_at_ms: Some(expires_at_ms),
            }
        }
        Some((owner_id, expires_at_ms)) => ServeLeaseSnapshot {
            state: ServeLeaseState::Stale,
            owner_id: Some(owner_id),
            expires_at_ms: Some(expires_at_ms),
        },
    }
}

fn serve_owner_is_active(owner_id: &str) -> bool {
    let Some(pid_raw) = owner_id.strip_prefix(SERVE_OWNER_ID_PREFIX) else {
        return true;
    };
    let Ok(pid) = pid_raw.parse::<u32>() else {
        return true;
    };

    let Ok(output) = Command::new("ps").arg("-p").arg(pid.to_string()).output() else {
        return true;
    };
    output.status.success() && String::from_utf8_lossy(&output.stdout).lines().count() > 1
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
