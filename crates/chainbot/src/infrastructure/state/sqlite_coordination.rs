//! [INPUT]
//! State layout paths, sqlite lease/token rows, and trigger record/snapshot coordination payloads.
//!
//! [OUTPUT]
//! Persists serve lease ownership and trigger coordination tokens with SQLite-backed migration/recovery semantics.
//!
//! [ROLE]
//! Owns SQLite-only coordination implementation for runtime-state lease and token boundaries.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};

use crate::domain::state::{
    LeaseAcquireResult, ServeLeaseGrant, ServeLeaseSnapshot, ServeLeaseState, TriggerEventRecord,
    TriggerSnapshotRecord, SERVE_OWNER_ID_PREFIX,
};

use super::file_store::StateLayout;

const SERVE_LEASE_KEY: &str = "serve";
const SQLITE_COORDINATION_SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoordinationTokenKind {
    Dedup,
    Cooldown,
}

#[derive(Debug)]
pub struct CoordinationStore {
    db_path: PathBuf,
    connection: Connection,
}

#[derive(Debug)]
pub enum CoordinationError {
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
}

impl CoordinationTokenKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dedup => "dedup",
            Self::Cooldown => "cooldown",
        }
    }
}

impl CoordinationStore {
    pub fn inspect_existing_serve_lease(
        layout: &StateLayout,
        now_ms: i64,
    ) -> Result<ServeLeaseSnapshot, CoordinationError> {
        if !layout.coordination_db_path.exists() {
            return Ok(ServeLeaseSnapshot {
                state: ServeLeaseState::Idle,
                owner_id: None,
                expires_at_ms: None,
            });
        }

        let connection = Connection::open_with_flags(
            &layout.coordination_db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|source| CoordinationError::Sqlite {
            path: layout.coordination_db_path.clone(),
            operation: "open sqlite database in read-only mode",
            source,
        })?;

        inspect_serve_lease_snapshot(&connection, &layout.coordination_db_path, now_ms)
    }

    pub fn open(layout: &StateLayout, now_ms: i64) -> Result<Self, CoordinationError> {
        create_dir_all_coordination(&layout.state_root)?;

        let mut connection = Connection::open(&layout.coordination_db_path).map_err(|source| {
            CoordinationError::Sqlite {
                path: layout.coordination_db_path.clone(),
                operation: "open sqlite database",
                source,
            }
        })?;

        connection
            .busy_timeout(Duration::from_millis(1_000))
            .map_err(|source| CoordinationError::Sqlite {
                path: layout.coordination_db_path.clone(),
                operation: "set sqlite busy timeout",
                source,
            })?;

        run_sqlite_migrations(&mut connection, &layout.coordination_db_path, now_ms)?;

        Ok(Self {
            db_path: layout.coordination_db_path.clone(),
            connection,
        })
    }

    pub fn database_path(&self) -> &Path {
        &self.db_path
    }

    pub fn applied_migration_versions(&self) -> Result<Vec<i64>, CoordinationError> {
        let mut statement = self
            .connection
            .prepare("SELECT version FROM schema_migrations ORDER BY version ASC")
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "prepare migration versions query",
                source,
            })?;

        let rows = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "query migration versions",
                source,
            })?;

        let mut versions = Vec::new();
        for row in rows {
            let version = row.map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "read migration version row",
                source,
            })?;
            versions.push(version);
        }

        Ok(versions)
    }

    pub fn inspect_serve_lease(
        &self,
        now_ms: i64,
    ) -> Result<ServeLeaseSnapshot, CoordinationError> {
        inspect_serve_lease_snapshot(&self.connection, &self.db_path, now_ms)
    }

    pub fn try_acquire_serve_lease(
        &mut self,
        owner_id: &str,
        now_ms: i64,
        lease_ttl_ms: i64,
    ) -> Result<LeaseAcquireResult, CoordinationError> {
        let expires_at_ms = now_ms.saturating_add(lease_ttl_ms.max(1));

        let transaction =
            self.connection
                .transaction()
                .map_err(|source| CoordinationError::Sqlite {
                    path: self.db_path.clone(),
                    operation: "start lease transaction",
                    source,
                })?;

        let current_lease = transaction
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
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "read current serve lease",
                source,
            })?;

        let acquire_result = match current_lease {
            Some((current_owner, current_expires_at_ms, _))
                if current_expires_at_ms > now_ms
                    && current_owner != owner_id
                    && serve_lease_owner_is_active(&current_owner) =>
            {
                LeaseAcquireResult::Rejected {
                    current_owner,
                    expires_at_ms: current_expires_at_ms,
                }
            }
            Some((current_owner, current_expires_at_ms, generation))
                if current_expires_at_ms > now_ms && current_owner == owner_id =>
            {
                write_lease_row(
                    &transaction,
                    owner_id,
                    now_ms,
                    expires_at_ms,
                    generation,
                    &self.db_path,
                )?;
                LeaseAcquireResult::Renewed {
                    grant: ServeLeaseGrant {
                        owner_id: owner_id.to_owned(),
                        generation: u64::try_from(generation).unwrap_or(0),
                        expires_at_ms,
                    },
                }
            }
            Some((_, _, generation)) => {
                let generation = generation.saturating_add(1);
                write_lease_row(
                    &transaction,
                    owner_id,
                    now_ms,
                    expires_at_ms,
                    generation,
                    &self.db_path,
                )?;
                LeaseAcquireResult::Acquired {
                    grant: ServeLeaseGrant {
                        owner_id: owner_id.to_owned(),
                        generation: u64::try_from(generation).unwrap_or(0),
                        expires_at_ms,
                    },
                }
            }
            None => {
                write_lease_row(
                    &transaction,
                    owner_id,
                    now_ms,
                    expires_at_ms,
                    0,
                    &self.db_path,
                )?;
                LeaseAcquireResult::Acquired {
                    grant: ServeLeaseGrant {
                        owner_id: owner_id.to_owned(),
                        generation: 0,
                        expires_at_ms,
                    },
                }
            }
        };

        transaction
            .commit()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "commit lease transaction",
                source,
            })?;

        Ok(acquire_result)
    }

    pub fn release_serve_lease(&mut self, owner_id: &str) -> Result<bool, CoordinationError> {
        let rows = self
            .connection
            .execute(
                "UPDATE serve_leases
                 SET expires_at_ms = 0
                 WHERE lease_key = ?1 AND owner_id = ?2",
                params![SERVE_LEASE_KEY, owner_id],
            )
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "release serve lease",
                source,
            })?;

        Ok(rows > 0)
    }

    pub fn mark_dedup_if_new(
        &mut self,
        dedup_key: &str,
        window_ms: i64,
        now_ms: i64,
    ) -> Result<bool, CoordinationError> {
        self.claim_token_if_ready(CoordinationTokenKind::Dedup, dedup_key, window_ms, now_ms)
    }

    pub fn begin_cooldown_if_ready(
        &mut self,
        cooldown_key: &str,
        duration_ms: i64,
        now_ms: i64,
    ) -> Result<bool, CoordinationError> {
        self.claim_token_if_ready(
            CoordinationTokenKind::Cooldown,
            cooldown_key,
            duration_ms,
            now_ms,
        )
    }

    pub fn dedup_is_ready(&self, dedup_key: &str, now_ms: i64) -> Result<bool, CoordinationError> {
        self.token_is_ready(CoordinationTokenKind::Dedup, dedup_key, now_ms)
    }

    pub fn cooldown_is_ready(
        &self,
        cooldown_key: &str,
        now_ms: i64,
    ) -> Result<bool, CoordinationError> {
        self.token_is_ready(CoordinationTokenKind::Cooldown, cooldown_key, now_ms)
    }

    pub fn rebuild_trigger_record_coordination(
        &mut self,
        records: &[TriggerEventRecord],
        now_ms: i64,
    ) -> Result<(), CoordinationError> {
        let transaction =
            self.connection
                .transaction()
                .map_err(|source| CoordinationError::Sqlite {
                    path: self.db_path.clone(),
                    operation: "start trigger coordination rebuild transaction",
                    source,
                })?;

        transaction
            .execute("DELETE FROM coordination_tokens", [])
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "clear coordination tokens before rebuild",
                source,
            })?;

        for record in records {
            upsert_trigger_record_coordination(&transaction, &self.db_path, record, now_ms)?;
        }

        transaction
            .commit()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "commit trigger coordination rebuild transaction",
                source,
            })
    }

    pub fn rebuild_trigger_snapshot_coordination(
        &mut self,
        snapshots: &[TriggerSnapshotRecord],
        now_ms: i64,
    ) -> Result<(), CoordinationError> {
        let transaction =
            self.connection
                .transaction()
                .map_err(|source| CoordinationError::Sqlite {
                    path: self.db_path.clone(),
                    operation: "start trigger snapshot coordination rebuild transaction",
                    source,
                })?;

        transaction
            .execute("DELETE FROM coordination_tokens", [])
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "clear coordination tokens before snapshot rebuild",
                source,
            })?;

        for snapshot in snapshots {
            for token in &snapshot.dedup_tokens {
                upsert_coordination_token_if_unexpired(
                    &transaction,
                    &self.db_path,
                    CoordinationTokenKind::Dedup,
                    Some(token.key.as_str()),
                    Some(token.expires_at_ms),
                    now_ms,
                )?;
            }
            for token in &snapshot.cooldown_tokens {
                upsert_coordination_token_if_unexpired(
                    &transaction,
                    &self.db_path,
                    CoordinationTokenKind::Cooldown,
                    Some(token.key.as_str()),
                    Some(token.expires_at_ms),
                    now_ms,
                )?;
            }
        }

        transaction
            .commit()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "commit trigger snapshot coordination rebuild transaction",
                source,
            })
    }

    pub fn apply_trigger_record_coordination(
        &mut self,
        record: &TriggerEventRecord,
        now_ms: i64,
    ) -> Result<(), CoordinationError> {
        let transaction =
            self.connection
                .transaction()
                .map_err(|source| CoordinationError::Sqlite {
                    path: self.db_path.clone(),
                    operation: "start trigger coordination write transaction",
                    source,
                })?;

        upsert_trigger_record_coordination(&transaction, &self.db_path, record, now_ms)?;

        transaction
            .commit()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "commit trigger coordination write transaction",
                source,
            })
    }

    fn claim_token_if_ready(
        &mut self,
        token_kind: CoordinationTokenKind,
        token_key: &str,
        duration_ms: i64,
        now_ms: i64,
    ) -> Result<bool, CoordinationError> {
        let next_expires_at_ms = now_ms.saturating_add(duration_ms.max(0));

        let transaction =
            self.connection
                .transaction()
                .map_err(|source| CoordinationError::Sqlite {
                    path: self.db_path.clone(),
                    operation: "start token transaction",
                    source,
                })?;

        let current_expires_at = transaction
            .query_row(
                "SELECT expires_at_ms FROM coordination_tokens WHERE token_kind = ?1 AND token_key = ?2",
                params![token_kind.as_str(), token_key],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "read coordination token",
                source,
            })?;

        if let Some(expires_at_ms) = current_expires_at {
            if expires_at_ms > now_ms {
                transaction
                    .commit()
                    .map_err(|source| CoordinationError::Sqlite {
                        path: self.db_path.clone(),
                        operation: "commit token read transaction",
                        source,
                    })?;
                return Ok(false);
            }
        }

        transaction
            .execute(
                "INSERT INTO coordination_tokens (token_kind, token_key, expires_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(token_kind, token_key)
                 DO UPDATE SET expires_at_ms = excluded.expires_at_ms, updated_at_ms = excluded.updated_at_ms",
                params![token_kind.as_str(), token_key, next_expires_at_ms, now_ms],
            )
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "upsert coordination token",
                source,
            })?;

        transaction
            .commit()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "commit token transaction",
                source,
            })?;

        Ok(true)
    }

    fn token_is_ready(
        &self,
        token_kind: CoordinationTokenKind,
        token_key: &str,
        now_ms: i64,
    ) -> Result<bool, CoordinationError> {
        let current_expires_at = self
            .connection
            .query_row(
                "SELECT expires_at_ms FROM coordination_tokens WHERE token_kind = ?1 AND token_key = ?2",
                params![token_kind.as_str(), token_key],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "read coordination token readiness",
                source,
            })?;

        Ok(current_expires_at.is_none_or(|expires_at_ms| expires_at_ms <= now_ms))
    }
}

impl Display for CoordinationError {
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
        }
    }
}

impl Error for CoordinationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Sqlite { source, .. } => Some(source),
        }
    }
}

fn create_dir_all_coordination(path: &Path) -> Result<(), CoordinationError> {
    std::fs::create_dir_all(path).map_err(|source| CoordinationError::Io {
        path: path.to_path_buf(),
        operation: "create directory",
        source,
    })
}

fn write_lease_row(
    transaction: &rusqlite::Transaction<'_>,
    owner_id: &str,
    acquired_at_ms: i64,
    expires_at_ms: i64,
    generation: i64,
    db_path: &Path,
) -> Result<(), CoordinationError> {
    transaction
        .execute(
            "INSERT INTO serve_leases (lease_key, owner_id, acquired_at_ms, expires_at_ms, generation)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(lease_key)
             DO UPDATE SET owner_id = excluded.owner_id,
                           acquired_at_ms = excluded.acquired_at_ms,
                           expires_at_ms = excluded.expires_at_ms,
                           generation = excluded.generation",
            params![SERVE_LEASE_KEY, owner_id, acquired_at_ms, expires_at_ms, generation],
        )
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "upsert serve lease",
            source,
        })?;
    Ok(())
}

fn upsert_trigger_record_coordination(
    transaction: &rusqlite::Transaction<'_>,
    db_path: &Path,
    record: &TriggerEventRecord,
    now_ms: i64,
) -> Result<(), CoordinationError> {
    upsert_coordination_token_if_unexpired(
        transaction,
        db_path,
        CoordinationTokenKind::Dedup,
        record.dedup_key.as_deref(),
        record.dedup_expires_at_ms,
        now_ms,
    )?;
    upsert_coordination_token_if_unexpired(
        transaction,
        db_path,
        CoordinationTokenKind::Cooldown,
        record.cooldown_key.as_deref(),
        record.cooldown_expires_at_ms,
        now_ms,
    )?;
    Ok(())
}

fn upsert_coordination_token_if_unexpired(
    transaction: &rusqlite::Transaction<'_>,
    db_path: &Path,
    token_kind: CoordinationTokenKind,
    token_key: Option<&str>,
    expires_at_ms: Option<i64>,
    now_ms: i64,
) -> Result<(), CoordinationError> {
    let (Some(token_key), Some(expires_at_ms)) = (token_key, expires_at_ms) else {
        return Ok(());
    };
    if expires_at_ms <= now_ms {
        return Ok(());
    }

    transaction
        .execute(
            "INSERT INTO coordination_tokens (token_kind, token_key, expires_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(token_kind, token_key)
             DO UPDATE SET expires_at_ms = CASE
                                WHEN coordination_tokens.expires_at_ms > excluded.expires_at_ms
                                THEN coordination_tokens.expires_at_ms
                                ELSE excluded.expires_at_ms
                            END,
                            updated_at_ms = excluded.updated_at_ms",
            params![token_kind.as_str(), token_key, expires_at_ms, now_ms],
        )
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "upsert trigger coordination token",
            source,
        })?;

    Ok(())
}

fn run_sqlite_migrations(
    connection: &mut Connection,
    db_path: &Path,
    now_ms: i64,
) -> Result<(), CoordinationError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at_ms INTEGER NOT NULL
            );",
        )
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "create schema migration table",
            source,
        })?;

    let transaction = connection
        .transaction()
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "start migration transaction",
            source,
        })?;

    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS serve_leases (
                lease_key TEXT PRIMARY KEY,
                owner_id TEXT NOT NULL,
                acquired_at_ms INTEGER NOT NULL,
                expires_at_ms INTEGER NOT NULL,
                generation INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS coordination_tokens (
                token_kind TEXT NOT NULL,
                token_key TEXT NOT NULL,
                expires_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                PRIMARY KEY (token_kind, token_key)
            );",
        )
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "apply coordination schema migration",
            source,
        })?;

    let has_generation = transaction
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('serve_leases') WHERE name = 'generation'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "inspect coordination lease generation migration",
            source,
        })?
        > 0;
    if !has_generation {
        transaction
            .execute(
                "ALTER TABLE serve_leases ADD COLUMN generation INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(|source| CoordinationError::Sqlite {
                path: db_path.to_path_buf(),
                operation: "apply coordination lease generation migration",
                source,
            })?;
    }

    for version in 1..=SQLITE_COORDINATION_SCHEMA_VERSION {
        transaction
            .execute(
                "INSERT OR IGNORE INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
                params![version, now_ms],
            )
            .map_err(|source| CoordinationError::Sqlite {
                path: db_path.to_path_buf(),
                operation: "record coordination schema migration",
                source,
            })?;
    }

    transaction
        .commit()
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "commit migration transaction",
            source,
        })
}

fn inspect_serve_lease_snapshot(
    connection: &Connection,
    db_path: &Path,
    now_ms: i64,
) -> Result<ServeLeaseSnapshot, CoordinationError> {
    let current_lease = connection
        .query_row(
            "SELECT owner_id, expires_at_ms FROM serve_leases WHERE lease_key = ?1",
            params![SERVE_LEASE_KEY],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "inspect current serve lease",
            source,
        })?;

    let snapshot = match current_lease {
        None => ServeLeaseSnapshot {
            state: ServeLeaseState::Idle,
            owner_id: None,
            expires_at_ms: None,
        },
        Some((owner_id, expires_at_ms))
            if expires_at_ms > now_ms && serve_lease_owner_is_active(&owner_id) =>
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
    };

    Ok(snapshot)
}

fn serve_lease_owner_is_active(owner_id: &str) -> bool {
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
