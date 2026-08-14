//! Durable, project-scoped Queen data (Phase 3.6–3.7).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, types::Type, Connection, OptionalExtension, Row, TransactionBehavior};
use serde::Serialize;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

const MAX_PINS_PER_PROJECT: i64 = 256;
const MAX_NOTES_PER_PROJECT: i64 = 10_000;
const MAX_PIN_KEY_BYTES: usize = 128;
const MAX_PIN_VALUE_BYTES: usize = 16 * 1024;
const MAX_NOTE_TITLE_BYTES: usize = 256;
const MAX_NOTE_BODY_BYTES: usize = 64 * 1024;
const MAX_TAGS: usize = 32;
const MAX_TAG_BYTES: usize = 64;
const MAX_MESSAGES_PER_PROJECT: i64 = 50_000;
const MAX_MAILBOX_BYTES: usize = 128;
const MAX_MESSAGE_SUBJECT_BYTES: usize = 256;
const MAX_MESSAGE_BODY_BYTES: usize = 64 * 1024;

/// Stage A-6: the `workflow_runs.state` values that mean "this run is over".
/// The single source of truth for both the Rust-side gate
/// (`is_terminal_workflow_state`) and the SQL below, which binds these three
/// strings as parameters rather than spelling them out a second time.
///
/// Mirrors `orchestrator::workflow_state_wire` for
/// `WorkflowState::{Succeeded, Failed, Cancelled}` — the states
/// `evict_terminal` already treats as terminal in memory. It is deliberately
/// an ALLOW-list and not a deny-list of `'running'`: a value this build does
/// not recognise (an older or newer schema, a hand-edited row) is treated as
/// still-live and therefore never deleted. Retention must fail towards
/// keeping rows, because the row it would wrongly delete is a resumable run.
const TERMINAL_WORKFLOW_STATES: [&str; 3] = ["succeeded", "failed", "cancelled"];

/// Stage A-6: how many *terminal* runs one project keeps in `workflow_runs`.
/// Non-terminal runs are never counted and never deleted (see
/// `prune_terminal_workflow_runs`), so this is a cap on HISTORY, not on
/// concurrency.
///
/// Until Stage A-6 this table had neither a cap nor a DELETE anywhere — the
/// only table in this store without one — so it grew monotonically for the
/// life of the install.
///
/// Where 500 comes from (both bounds are derived, not guessed; the byte
/// figure inside the upper bound is an ASSUMPTION and is called out as such):
///
/// - **Lower bound — 100.** `orchestrator::REGISTRY_TERMINAL_CAP` is the
///   number of terminal runs the in-memory registry keeps. A durable store
///   narrower than a volatile one is nonsense: the DB must be able to hold at
///   least everything the running app is still willing to show. 500 is 5x
///   that, so the DB window is strictly the wider of the two.
/// - **Upper bound — bounded file growth.** One row carries the run's WHOLE
///   `steps_json` (see `WORKFLOW_RUNS_SCHEMA_SQL`), so rows here are an order
///   of magnitude heavier than a pin or an inbox row. At a deliberately
///   pessimistic 1 KiB/row — room for ~9 JSON keys per step across a handful
///   of steps plus a few hundred bytes of `error` text — 500 rows is ~0.5 MiB
///   per project. That is far below the worst case the neighbouring caps
///   already permit (`MAX_NOTES_PER_PROJECT` x `MAX_NOTE_BODY_BYTES` alone is
///   ~640 MiB), so 500 is conservative in the company it keeps. **The 1 KiB
///   is an estimate: real `steps_json` sizes have not been measured.**
/// - **Time axis.** Unattended scheduling (the `every: hour` work on the
///   `feat/schedule-5.0.8` branch, not in this build) produces 24 rows/day
///   from one workflow, so 500 is ~20 days of hourly runs, or well over a
///   year of daily ones. "A few weeks of history survives a machine left
///   running unattended" is the property the number is chosen for.
///
/// Like `orchestrator::STREAM_MAX_UNITS`, this is an initial value with a
/// derivation but no field measurement behind it. Phase 5.6.0 (the schema
/// split) makes rows small and cheap, and should revisit it then.
const MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT: i64 = 500;

/// Phase 5.0.1: `workflow_runs` schema, shared verbatim by all three
/// migration paths below (fresh v0 database, v1->v3, v2->v3) so the
/// table/index text is defined exactly once. `IF NOT EXISTS` keeps every
/// path idempotent against a partially-migrated db (same L12a discipline as
/// the existing tables). `error` is populated only by
/// `mark_workflow_abandoned` — every other column mirrors
/// `orchestrator::WorkflowRun` closely enough to round-trip it.
const WORKFLOW_RUNS_SCHEMA_SQL: &str = "
                 CREATE TABLE IF NOT EXISTS workflow_runs (
                   run_id TEXT PRIMARY KEY,
                   project_dir TEXT NOT NULL,
                   name TEXT NOT NULL,
                   state TEXT NOT NULL,
                   started_at_ms INTEGER NOT NULL,
                   ended_at_ms INTEGER,
                   steps_json TEXT NOT NULL,
                   error TEXT
                 );
                 CREATE INDEX IF NOT EXISTS workflow_runs_project_state
                   ON workflow_runs(project_dir, state, started_at_ms DESC);";

/// One persisted `workflow_runs` row (Phase 5.0.1). A plain,
/// orchestrator-independent shape — `steps_json` is opaque to this module;
/// only the orchestrator knows how to decode it back into `Vec<StepOutcome>`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunRow {
    pub run_id: String,
    pub name: String,
    pub state: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub steps_json: String,
    pub project_dir: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    pub key: String,
    pub value: String,
    pub revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InboxMessage {
    pub id: i64,
    pub sender: String,
    pub recipient: String,
    pub subject: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to_id: Option<i64>,
    pub root_message_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acknowledged_at_ms: Option<i64>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboxWait {
    Messages(Vec<InboxMessage>),
    TimedOut,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct InboxWaitOptions {
    pub mailbox: String,
    pub after_id: i64,
    pub include_acknowledged: bool,
    pub limit: u32,
    pub timeout: Duration,
}

/// `(project_id, mailbox)` — the exact pair `send_inbox`/`reply_inbox` deliver
/// to, so a waiter is only ever woken by traffic addressed to it.
type MailboxKey = (String, String);

pub struct QueenStore {
    connection: Mutex<Connection>,
    /// Store-wide inbox generation. Superseded by `mailbox_waiters` for waking
    /// `await_inbox`, but kept as a coarse "something arrived" signal that the
    /// tests observe directly.
    inbox_generation: watch::Sender<u64>,
    /// One watch channel per mailbox that currently has at least one waiter.
    /// Entries are created only by `await_inbox` (via `subscribe_mailbox`) and
    /// removed by `MailboxSubscription::drop`, so an idle store holds none.
    ///
    /// LOCK ORDER: `connection` -> `mailbox_waiters`, one direction only.
    /// `send_inbox`/`reply_inbox` call `notify_inbox` while still holding the
    /// `connection` guard, so taking `self.lock()` while holding
    /// `mailbox_waiters` anywhere would deadlock. Neither `subscribe_mailbox`
    /// nor `MailboxSubscription::drop` touches `connection`.
    mailbox_waiters: Mutex<HashMap<MailboxKey, Arc<watch::Sender<u64>>>>,
}

/// RAII handle over one entry of `QueenStore::mailbox_waiters`, owned by a
/// single `await_inbox` call. Dropping it garbage-collects the entry once the
/// last waiter on that mailbox is gone.
struct MailboxSubscription<'a> {
    store: &'a QueenStore,
    key: MailboxKey,
    /// The exact channel this subscription was handed, kept so `Drop` can tell
    /// it apart from a later channel registered under the same key.
    sender: Arc<watch::Sender<u64>>,
    /// `Option` only so `Drop` can release it before inspecting the map.
    receiver: Option<watch::Receiver<u64>>,
}

impl MailboxSubscription<'_> {
    fn receiver_mut(&mut self) -> &mut watch::Receiver<u64> {
        self.receiver
            .as_mut()
            .expect("mailbox subscription receiver is only taken in Drop")
    }
}

impl Drop for MailboxSubscription<'_> {
    fn drop(&mut self) {
        // (a) Release our own receiver first, so `receiver_count()` below
        //     reflects the other waiters only.
        drop(self.receiver.take());
        // (b) Decide under the map lock, so a concurrent `subscribe_mailbox`
        //     cannot slip a new receiver in between the check and the remove.
        //     Takes no other lock — see the LOCK ORDER note on
        //     `QueenStore::mailbox_waiters`.
        let mut waiters = self.store.lock_mailbox_waiters();
        // (c) Remove only if the mapped channel is still *ours* and nobody is
        //     listening. Without the `ptr_eq` check, a subscription that lost
        //     a race (its entry already removed and replaced) would delete a
        //     fresh channel that another waiter is currently blocked on, and
        //     that waiter would never be woken again.
        let is_ours_and_idle = waiters.get(&self.key).is_some_and(|current| {
            Arc::ptr_eq(current, &self.sender) && current.receiver_count() == 0
        });
        if is_ours_and_idle {
            waiters.remove(&self.key);
        }
    }
}

impl QueenStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "Queen database path has no parent".to_string())?;
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create Queen data directory: {e}"))?;
        let connection = Connection::open(path)
            .map_err(|e| format!("cannot open Queen database {}: {e}", path.display()))?;
        let store = Self::from_connection(connection, true)?;
        // Stage A-6, v0.5.9 §2.4: retention only ever ran when a run REACHED a
        // terminal state, so a database fattened by a pre-A-6 build never
        // shrank in a project that had stopped running workflows — the exact
        // installs the cap was written for kept every row forever. One sweep
        // per process start closes that. Best-effort: a store that opened is
        // usable, and refusing to start over a failed tidy would be a strictly
        // worse outcome than the oversized table it was tidying.
        //
        // Deliberately NO `VACUUM`: reclaiming the freed pages means rewriting
        // the whole file on the startup path, and SQLite reuses free pages for
        // subsequent writes anyway, so the growth this bounds stops either way.
        if let Err(error) = store.prune_every_projects_terminal_workflow_runs() {
            eprintln!("queen: startup workflow-run retention sweep failed: {error}");
        }
        Ok(store)
    }

    /// Test-support constructor, also used by the team_presets tests.
    #[cfg(test)]
    pub(crate) fn open_in_memory() -> Result<Self, String> {
        let connection = Connection::open_in_memory()
            .map_err(|e| format!("cannot open in-memory Queen database: {e}"))?;
        Self::from_connection(connection, false)
    }

    fn from_connection(connection: Connection, persistent: bool) -> Result<Self, String> {
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|e| format!("cannot configure Queen database timeout: {e}"))?;
        if persistent {
            connection
                .pragma_update(None, "journal_mode", "WAL")
                .map_err(|e| format!("cannot enable Queen database WAL: {e}"))?;
            connection
                .pragma_update(None, "synchronous", "NORMAL")
                .map_err(|e| format!("cannot configure Queen database sync mode: {e}"))?;
        }
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| format!("cannot configure Queen database: {e}"))?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|e| format!("cannot read Queen database version: {e}"))?;
        if version > 3 {
            return Err(format!(
                "unsupported Queen database version {version} (expected 3)"
            ));
        }
        if version == 0 {
            let schema_result = connection.execute_batch(&format!(
                "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS pins (
                   project_dir TEXT NOT NULL,
                   pin_key TEXT NOT NULL,
                   value TEXT NOT NULL,
                   revision INTEGER NOT NULL,
                   created_at_ms INTEGER NOT NULL,
                   updated_at_ms INTEGER NOT NULL,
                   PRIMARY KEY (project_dir, pin_key)
                 );
                 CREATE TABLE IF NOT EXISTS notes (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   project_dir TEXT NOT NULL,
                   title TEXT NOT NULL,
                   body TEXT NOT NULL,
                   tags_json TEXT NOT NULL,
                   revision INTEGER NOT NULL,
                   created_at_ms INTEGER NOT NULL,
                   updated_at_ms INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS notes_project_updated
                   ON notes(project_dir, updated_at_ms DESC, id DESC);
                 CREATE TABLE IF NOT EXISTS inbox_messages (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   project_dir TEXT NOT NULL,
                   sender TEXT NOT NULL,
                   recipient TEXT NOT NULL,
                   subject TEXT NOT NULL,
                   body TEXT NOT NULL,
                   in_reply_to_id INTEGER,
                   root_message_id INTEGER,
                   acknowledged_at_ms INTEGER,
                   created_at_ms INTEGER NOT NULL,
                   FOREIGN KEY (in_reply_to_id) REFERENCES inbox_messages(id),
                   FOREIGN KEY (root_message_id) REFERENCES inbox_messages(id)
                 );
                 CREATE INDEX IF NOT EXISTS inbox_recipient_id
                   ON inbox_messages(project_dir, recipient, id ASC);
                 CREATE INDEX IF NOT EXISTS inbox_root_id
                   ON inbox_messages(project_dir, root_message_id, id ASC);
                 {WORKFLOW_RUNS_SCHEMA_SQL}
                 PRAGMA user_version = 3;
                 COMMIT;",
            ));
            if let Err(error) = schema_result {
                let _ = connection.execute_batch("ROLLBACK;");
                return Err(format!("cannot initialize Queen database: {error}"));
            }
        } else if version == 1 {
            // `IF NOT EXISTS` so a partially-migrated db (user_version still 1
            // but the table/indexes already present) migrates idempotently
            // instead of hard-failing `open` with "table already exists" (L12a).
            let migration_result = connection.execute_batch(&format!(
                "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS inbox_messages (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   project_dir TEXT NOT NULL,
                   sender TEXT NOT NULL,
                   recipient TEXT NOT NULL,
                   subject TEXT NOT NULL,
                   body TEXT NOT NULL,
                   in_reply_to_id INTEGER,
                   root_message_id INTEGER,
                   acknowledged_at_ms INTEGER,
                   created_at_ms INTEGER NOT NULL,
                   FOREIGN KEY (in_reply_to_id) REFERENCES inbox_messages(id),
                   FOREIGN KEY (root_message_id) REFERENCES inbox_messages(id)
                 );
                 CREATE INDEX IF NOT EXISTS inbox_recipient_id
                   ON inbox_messages(project_dir, recipient, id ASC);
                 CREATE INDEX IF NOT EXISTS inbox_root_id
                   ON inbox_messages(project_dir, root_message_id, id ASC);
                 {WORKFLOW_RUNS_SCHEMA_SQL}
                 PRAGMA user_version = 3;
                 COMMIT;",
            ));
            if let Err(error) = migration_result {
                let _ = connection.execute_batch("ROLLBACK;");
                return Err(format!(
                    "cannot migrate Queen database to version 3: {error}"
                ));
            }
        } else if version == 2 {
            // Phase 5.0.1: pins/notes/inbox_messages already present (v2) —
            // add just the new `workflow_runs` table + index and bump
            // straight to v3. `IF NOT EXISTS` keeps this idempotent against a
            // partially-migrated db, same L12a discipline as the v1 branch.
            let workflow_migration_result = connection.execute_batch(&format!(
                "BEGIN IMMEDIATE;
                 {WORKFLOW_RUNS_SCHEMA_SQL}
                 PRAGMA user_version = 3;
                 COMMIT;",
            ));
            if let Err(error) = workflow_migration_result {
                let _ = connection.execute_batch("ROLLBACK;");
                return Err(format!(
                    "cannot migrate Queen database to version 3: {error}"
                ));
            }
        }
        let (inbox_generation, _) = watch::channel(0);
        Ok(Self {
            connection: Mutex::new(connection),
            inbox_generation,
            mailbox_waiters: Mutex::new(HashMap::new()),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        match self.connection.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// See the LOCK ORDER note on `QueenStore::mailbox_waiters`: never take
    /// `self.lock()` while the returned guard is alive.
    fn lock_mailbox_waiters(&self) -> MutexGuard<'_, HashMap<MailboxKey, Arc<watch::Sender<u64>>>> {
        match self.mailbox_waiters.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Number of live per-mailbox watch entries. Test-only: asserts that the
    /// map is self-cleaning, i.e. that waiters do not leak channels.
    #[cfg(test)]
    pub(crate) fn mailbox_waiter_count(&self) -> usize {
        self.lock_mailbox_waiters().len()
    }

    /// Current generation of one mailbox's channel, or `None` when nobody is
    /// waiting on it. Test-only: lets a test assert that a waiter was *not*
    /// woken, which a plain timeout assertion cannot distinguish from a
    /// spurious wakeup that simply re-read an empty inbox.
    #[cfg(test)]
    pub(crate) fn mailbox_generation(&self, project: &Path, mailbox: &str) -> Option<u64> {
        let key = (project_id(project).ok()?, mailbox.to_string());
        let waiters = self.lock_mailbox_waiters();
        waiters.get(&key).map(|sender| *sender.borrow())
    }

    pub fn set_pin(
        &self,
        project: &Path,
        key: String,
        value: String,
        expected_revision: Option<i64>,
    ) -> Result<Pin, String> {
        let project = project_id(project)?;
        let key = validated_required("pin key", key, MAX_PIN_KEY_BYTES)?;
        validate_max("pin value", &value, MAX_PIN_VALUE_BYTES)?;
        let now = now_ms();
        let mut connection = self.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let current_revision: Option<i64> = transaction
            .query_row(
                "SELECT revision FROM pins WHERE project_dir = ?1 AND pin_key = ?2",
                params![project, key],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        match current_revision {
            None => {
                if expected_revision.is_some() {
                    return Err(format!(
                        "conflict: pin '{key}' no longer exists; refresh pins before retrying"
                    ));
                }
                enforce_limit(&transaction, "pins", &project, MAX_PINS_PER_PROJECT)?;
                transaction
                    .execute(
                        "INSERT INTO pins(
                           project_dir, pin_key, value, revision, created_at_ms, updated_at_ms
                         ) VALUES (?1, ?2, ?3, 1, ?4, ?4)",
                        params![project, key, value, now],
                    )
                    .map_err(db_error)?;
            }
            Some(current) => {
                if expected_revision != Some(current) {
                    return Err(format!(
                        "conflict: pin '{key}' is revision {current}; expectedRevision is required and must match"
                    ));
                }
                transaction
                    .execute(
                        "UPDATE pins SET value = ?3, revision = revision + 1, updated_at_ms = ?4
                         WHERE project_dir = ?1 AND pin_key = ?2 AND revision = ?5",
                        params![project, key, value, now, current],
                    )
                    .map_err(db_error)?;
            }
        }
        let pin = transaction
            .query_row(
                "SELECT pin_key, value, revision, created_at_ms, updated_at_ms
                 FROM pins WHERE project_dir = ?1 AND pin_key = ?2",
                params![project, key],
                pin_from_row,
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok(pin)
    }

    pub fn list_pins(&self, project: &Path) -> Result<Vec<Pin>, String> {
        let project = project_id(project)?;
        let connection = self.lock();
        let mut statement = connection
            .prepare(
                "SELECT pin_key, value, revision, created_at_ms, updated_at_ms
                 FROM pins WHERE project_dir = ?1 ORDER BY pin_key ASC",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map(params![project], pin_from_row)
            .map_err(db_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
    }

    pub fn delete_pin(
        &self,
        project: &Path,
        key: String,
        expected_revision: i64,
    ) -> Result<(), String> {
        let project = project_id(project)?;
        let key = validated_required("pin key", key, MAX_PIN_KEY_BYTES)?;
        validate_revision(expected_revision)?;
        let mut connection = self.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let deleted = transaction
            .execute(
                "DELETE FROM pins
                 WHERE project_dir = ?1 AND pin_key = ?2 AND revision = ?3",
                params![project, key, expected_revision],
            )
            .map_err(db_error)?
            > 0;
        if !deleted {
            let current: Option<i64> = transaction
                .query_row(
                    "SELECT revision FROM pins WHERE project_dir = ?1 AND pin_key = ?2",
                    params![project, key],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)?;
            return Err(match current {
                Some(revision) => {
                    format!("conflict: pin '{key}' is revision {revision}, not {expected_revision}")
                }
                None => format!("pin '{key}' not found"),
            });
        }
        transaction.commit().map_err(db_error)?;
        Ok(())
    }

    pub fn create_note(
        &self,
        project: &Path,
        title: String,
        body: String,
        tags: Vec<String>,
    ) -> Result<Note, String> {
        let project = project_id(project)?;
        let title = validated_required("note title", title, MAX_NOTE_TITLE_BYTES)?;
        validate_max("note body", &body, MAX_NOTE_BODY_BYTES)?;
        let tags = validated_tags(tags)?;
        let tags_json =
            serde_json::to_string(&tags).map_err(|e| format!("cannot encode note tags: {e}"))?;
        let now = now_ms();
        let mut connection = self.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        enforce_limit(&transaction, "notes", &project, MAX_NOTES_PER_PROJECT)?;
        transaction
            .execute(
                "INSERT INTO notes(
                   project_dir, title, body, tags_json, revision, created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)",
                params![project, title, body, tags_json, now],
            )
            .map_err(db_error)?;
        let id = transaction.last_insert_rowid();
        let note = get_note_from(&transaction, &project, id)?
            .ok_or_else(|| "note was inserted but could not be read back".to_string())?;
        transaction.commit().map_err(db_error)?;
        Ok(note)
    }

    pub fn list_notes(
        &self,
        project: &Path,
        query: Option<String>,
        limit: u32,
    ) -> Result<Vec<Note>, String> {
        let project = project_id(project)?;
        let query = query
            .map(|value| validated_required("note query", value, MAX_NOTE_TITLE_BYTES))
            .transpose()?;
        let limit = limit.clamp(1, 200) as i64;
        let connection = self.lock();
        let (sql, query_value) = if let Some(query) = query {
            (
                "SELECT id, title, body, tags_json, revision, created_at_ms, updated_at_ms
                 FROM notes
                 WHERE project_dir = ?1
                   AND (instr(lower(title), lower(?2)) > 0
                        OR instr(lower(body), lower(?2)) > 0
                        OR instr(lower(tags_json), lower(?2)) > 0)
                 ORDER BY updated_at_ms DESC, id DESC LIMIT ?3",
                Some(query),
            )
        } else {
            (
                "SELECT id, title, body, tags_json, revision, created_at_ms, updated_at_ms
                 FROM notes WHERE project_dir = ?1
                 ORDER BY updated_at_ms DESC, id DESC LIMIT ?3",
                None,
            )
        };
        let mut statement = connection.prepare(sql).map_err(db_error)?;
        let rows = statement
            .query_map(params![project, query_value, limit], note_from_row)
            .map_err(db_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
    }

    pub fn get_note(&self, project: &Path, id: i64) -> Result<Option<Note>, String> {
        validate_note_id(id)?;
        let project = project_id(project)?;
        get_note_from(&self.lock(), &project, id)
    }

    pub fn update_note(
        &self,
        project: &Path,
        id: i64,
        expected_revision: i64,
        title: Option<String>,
        body: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<Note, String> {
        validate_note_id(id)?;
        validate_revision(expected_revision)?;
        if title.is_none() && body.is_none() && tags.is_none() {
            return Err("update_note requires title, body, or tags".to_string());
        }
        let project = project_id(project)?;
        let title = title
            .map(|value| validated_required("note title", value, MAX_NOTE_TITLE_BYTES))
            .transpose()?;
        if let Some(body) = body.as_deref() {
            validate_max("note body", body, MAX_NOTE_BODY_BYTES)?;
        }
        let tags_json = tags
            .map(validated_tags)
            .transpose()?
            .map(|tags| serde_json::to_string(&tags))
            .transpose()
            .map_err(|e| format!("cannot encode note tags: {e}"))?;
        let mut connection = self.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let changed = transaction
            .execute(
                "UPDATE notes SET
                   title = COALESCE(?3, title),
                   body = COALESCE(?4, body),
                   tags_json = COALESCE(?5, tags_json),
                   revision = revision + 1,
                   updated_at_ms = ?6
                 WHERE project_dir = ?1 AND id = ?2 AND revision = ?7",
                params![
                    project,
                    id,
                    title,
                    body,
                    tags_json,
                    now_ms(),
                    expected_revision
                ],
            )
            .map_err(db_error)?;
        if changed == 0 {
            let current = get_note_from(&transaction, &project, id)?;
            return Err(match current {
                Some(note) => format!(
                    "conflict: note {id} is revision {}, not {expected_revision}",
                    note.revision
                ),
                None => format!("note {id} not found"),
            });
        }
        let note = get_note_from(&transaction, &project, id)?;
        let note = note.ok_or_else(|| format!("note {id} not found"))?;
        transaction.commit().map_err(db_error)?;
        Ok(note)
    }

    pub fn delete_note(
        &self,
        project: &Path,
        id: i64,
        expected_revision: i64,
    ) -> Result<(), String> {
        validate_note_id(id)?;
        validate_revision(expected_revision)?;
        let project = project_id(project)?;
        let mut connection = self.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let deleted = transaction
            .execute(
                "DELETE FROM notes
                 WHERE project_dir = ?1 AND id = ?2 AND revision = ?3",
                params![project, id, expected_revision],
            )
            .map_err(db_error)?
            > 0;
        if !deleted {
            let current = get_note_from(&transaction, &project, id)?;
            return Err(match current {
                Some(note) => format!(
                    "conflict: note {id} is revision {}, not {expected_revision}",
                    note.revision
                ),
                None => format!("note {id} not found"),
            });
        }
        transaction.commit().map_err(db_error)?;
        Ok(())
    }

    pub fn send_inbox(
        &self,
        project: &Path,
        sender: String,
        recipient: String,
        subject: String,
        body: String,
    ) -> Result<InboxMessage, String> {
        let project = project_id(project)?;
        let sender = validated_mailbox("sender", sender)?;
        let recipient = validated_mailbox("recipient", recipient)?;
        let subject = validated_required("message subject", subject, MAX_MESSAGE_SUBJECT_BYTES)?;
        let body = validated_required("message body", body, MAX_MESSAGE_BODY_BYTES)?;
        let now = now_ms();
        let mut connection = self.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        enforce_limit(
            &transaction,
            "inbox_messages",
            &project,
            MAX_MESSAGES_PER_PROJECT,
        )?;
        transaction
            .execute(
                "INSERT INTO inbox_messages(
                   project_dir, sender, recipient, subject, body,
                   in_reply_to_id, root_message_id, acknowledged_at_ms, created_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, ?6)",
                params![project, sender, recipient, subject, body, now],
            )
            .map_err(db_error)?;
        let id = transaction.last_insert_rowid();
        transaction
            .execute(
                "UPDATE inbox_messages SET root_message_id = id
                 WHERE project_dir = ?1 AND id = ?2",
                params![project, id],
            )
            .map_err(db_error)?;
        let message = get_inbox_from(&transaction, &project, id)?
            .ok_or_else(|| "inbox message was inserted but could not be read back".to_string())?;
        transaction.commit().map_err(db_error)?;
        self.notify_inbox(&project, &recipient);
        Ok(message)
    }

    pub fn list_inbox(
        &self,
        project: &Path,
        mailbox: String,
        after_id: i64,
        include_acknowledged: bool,
        limit: u32,
    ) -> Result<Vec<InboxMessage>, String> {
        if after_id < 0 {
            return Err("afterId must be zero or a positive integer".to_string());
        }
        let project = project_id(project)?;
        let mailbox = validated_mailbox("mailbox", mailbox)?;
        let limit = limit.clamp(1, 200) as i64;
        let connection = self.lock();
        let mut statement = connection
            .prepare(
                "SELECT id, sender, recipient, subject, body, in_reply_to_id,
                        root_message_id, acknowledged_at_ms, created_at_ms
                 FROM inbox_messages
                 WHERE project_dir = ?1 AND recipient = ?2 AND id > ?3
                   AND (?4 = 1 OR acknowledged_at_ms IS NULL)
                 ORDER BY id ASC LIMIT ?5",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map(
                params![
                    project,
                    mailbox,
                    after_id,
                    include_acknowledged as i64,
                    limit
                ],
                inbox_from_row,
            )
            .map_err(db_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
    }

    pub fn ack_inbox(
        &self,
        project: &Path,
        id: i64,
        recipient: String,
    ) -> Result<InboxMessage, String> {
        validate_message_id(id)?;
        let project = project_id(project)?;
        let recipient = validated_mailbox("recipient", recipient)?;
        let mut connection = self.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let current = get_inbox_from(&transaction, &project, id)?
            .ok_or_else(|| format!("inbox message {id} not found"))?;
        if current.recipient != recipient {
            return Err(format!(
                "inbox message {id} belongs to recipient '{}', not '{recipient}'",
                current.recipient
            ));
        }
        if current.acknowledged_at_ms.is_none() {
            transaction
                .execute(
                    "UPDATE inbox_messages SET acknowledged_at_ms = ?3
                     WHERE project_dir = ?1 AND id = ?2 AND acknowledged_at_ms IS NULL",
                    params![project, id, now_ms()],
                )
                .map_err(db_error)?;
        }
        let message = get_inbox_from(&transaction, &project, id)?
            .ok_or_else(|| format!("inbox message {id} not found"))?;
        transaction.commit().map_err(db_error)?;
        Ok(message)
    }

    /// Acknowledge every still-unacknowledged message a given `sender` put in
    /// somebody else's mailbox, and report how many that closed (Stage A-5).
    ///
    /// The sender-side counterpart of `ack_inbox`, which is recipient-side and
    /// takes one id at a time. It exists for one caller shape: a workflow run
    /// that ends without its agents ever answering — cancelled, or abandoned
    /// after a restart — leaves its kickoffs sitting unread and unacknowledged
    /// in the AGENT'S mailbox, where the next run's pane picks them up as live
    /// instructions (observed on hardware 2026-08-05, plan.md §6.14). The
    /// orchestrator cannot express that cleanup as a list of ids: the kickoff
    /// thread roots it records are `#[serde(skip)]`, so a run read back from
    /// the DB has none of them.
    ///
    /// `sender` is the whole selector, which is safe precisely because
    /// `orchestrator::workflow_mailbox` embeds the run id in it — no string
    /// other than that one run's own `queen:workflow/<name>/<run_id>` can
    /// match, so this can never acknowledge a sibling run's kickoff. Callers
    /// with a less specific sender get a less specific sweep; that is on them.
    ///
    /// Already-acknowledged messages are left exactly as they are (their
    /// original `acknowledged_at_ms` stands) and are not counted, so calling
    /// this twice on one run is a no-op the second time.
    ///
    /// COST: `inbox_messages` has no index on `sender`, so this is a scan of
    /// the project's messages (bounded by `MAX_MESSAGES_PER_PROJECT`). It runs
    /// once per cancel/abandon — never on the driver tick — so an index and
    /// the schema migration it would need are not worth it.
    pub fn ack_inbox_from_sender(&self, project: &Path, sender: String) -> Result<usize, String> {
        let project = project_id(project)?;
        let sender = validated_mailbox("sender", sender)?;
        let mut connection = self.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let acked = transaction
            .execute(
                "UPDATE inbox_messages SET acknowledged_at_ms = ?3
                 WHERE project_dir = ?1 AND sender = ?2 AND acknowledged_at_ms IS NULL",
                params![project, sender, now_ms()],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok(acked)
    }

    pub fn reply_inbox(
        &self,
        project: &Path,
        id: i64,
        sender: String,
        body: String,
    ) -> Result<InboxMessage, String> {
        validate_message_id(id)?;
        let project = project_id(project)?;
        let sender = validated_mailbox("sender", sender)?;
        let body = validated_required("message body", body, MAX_MESSAGE_BODY_BYTES)?;
        let mut connection = self.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let original = get_inbox_from(&transaction, &project, id)?
            .ok_or_else(|| format!("inbox message {id} not found"))?;
        if original.recipient != sender {
            return Err(format!(
                "only recipient '{}' can reply to inbox message {id}",
                original.recipient
            ));
        }
        enforce_limit(
            &transaction,
            "inbox_messages",
            &project,
            MAX_MESSAGES_PER_PROJECT,
        )?;
        let now = now_ms();
        transaction
            .execute(
                "INSERT INTO inbox_messages(
                   project_dir, sender, recipient, subject, body,
                   in_reply_to_id, root_message_id, acknowledged_at_ms, created_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8)",
                params![
                    project,
                    sender,
                    original.sender,
                    original.subject,
                    body,
                    original.id,
                    original.root_message_id,
                    now
                ],
            )
            .map_err(db_error)?;
        let reply_id = transaction.last_insert_rowid();
        transaction
            .execute(
                "UPDATE inbox_messages
                 SET acknowledged_at_ms = COALESCE(acknowledged_at_ms, ?3)
                 WHERE project_dir = ?1 AND id = ?2",
                params![project, id, now],
            )
            .map_err(db_error)?;
        let reply = get_inbox_from(&transaction, &project, reply_id)?
            .ok_or_else(|| "inbox reply was inserted but could not be read back".to_string())?;
        transaction.commit().map_err(db_error)?;
        self.notify_inbox(&project, &reply.recipient);
        Ok(reply)
    }

    pub async fn await_inbox(
        &self,
        project: &Path,
        options: InboxWaitOptions,
        cancellation: CancellationToken,
    ) -> Result<InboxWait, String> {
        let InboxWaitOptions {
            mailbox,
            after_id,
            include_acknowledged,
            limit,
            timeout,
        } = options;
        let project_key = project_id(project)?;
        let mailbox = validated_mailbox("mailbox", mailbox)?;
        // Subscribe BEFORE the first `list_inbox` below. This ordering is the
        // only thing preventing a lost wakeup: a message committed between the
        // read and the `select!` would otherwise notify a channel nobody is
        // listening to yet, and this call would sleep until its deadline even
        // though its message is already in the table. Because the subscription
        // exists first, `notify_inbox` either fires into our receiver (and
        // `changed()` returns at once) or it happened before we subscribed —
        // in which case its `COMMIT` also happened before our read, so the
        // first `list_inbox` already sees the message. Do NOT move this into
        // the loop.
        let mut subscription = self.subscribe_mailbox((project_key, mailbox.clone()));
        let changes = subscription.receiver_mut();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if cancellation.is_cancelled() {
                return Ok(InboxWait::Cancelled);
            }
            let messages = self.list_inbox(
                project,
                mailbox.clone(),
                after_id,
                include_acknowledged,
                limit,
            )?;
            if cancellation.is_cancelled() {
                return Ok(InboxWait::Cancelled);
            }
            if !messages.is_empty() {
                return Ok(InboxWait::Messages(messages));
            }

            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Ok(InboxWait::Cancelled),
                _ = tokio::time::sleep_until(deadline) => return Ok(InboxWait::TimedOut),
                changed = changes.changed() => {
                    if changed.is_err() {
                        return Err("Queen inbox notification channel closed".to_string());
                    }
                }
            }
        }
    }

    /// Bump the store-wide generation, then wake only the mailbox this message
    /// was addressed to. Callers pass the already-canonicalised project id and
    /// the already-validated recipient, so the key matches what `await_inbox`
    /// subscribed with.
    ///
    /// Called with the `connection` guard held (see the LOCK ORDER note on
    /// `QueenStore::mailbox_waiters`).
    fn notify_inbox(&self, project: &str, mailbox: &str) {
        self.inbox_generation
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        let waiters = self.lock_mailbox_waiters();
        // Deliberately a lookup and not an `entry()`: a mailbox nobody is
        // waiting on must not gain a channel here, otherwise every recipient
        // ever written to would leak one for the lifetime of the store.
        if let Some(sender) = waiters.get(&(project.to_string(), mailbox.to_string())) {
            sender.send_modify(|generation| *generation = generation.wrapping_add(1));
        }
    }

    /// Hand out a receiver for `key`, creating the shared channel on first use.
    /// The returned guard removes the entry again once the last waiter leaves.
    ///
    /// `subscribe()` is called *inside* the `mailbox_waiters` lock, not after
    /// releasing it. If it were called after, another waiter's `Drop` could
    /// run in the gap: it would see `receiver_count() == 0` (our subscription
    /// doesn't exist yet), remove the entry, and leave us subscribed to a
    /// channel nobody can look up via `notify_inbox` any more — a lost
    /// wakeup that only times out. So the invariant that guards the map isn't
    /// just the three conditions in `MailboxSubscription::drop` (ours, still
    /// mapped, idle); it also requires that subscribing to an existing
    /// channel is atomic with observing it under the same lock.
    fn subscribe_mailbox(&self, key: MailboxKey) -> MailboxSubscription<'_> {
        let (sender, receiver) = {
            // Never touches `connection` — see the LOCK ORDER note.
            let mut waiters = self.lock_mailbox_waiters();
            let sender = Arc::clone(
                waiters
                    .entry(key.clone())
                    .or_insert_with(|| Arc::new(watch::channel(0).0)),
            );
            // Subscribing here, still under the lock, is the atomicity that
            // rules out the lost-wakeup race described above.
            let receiver = sender.subscribe();
            (sender, receiver)
        };
        MailboxSubscription {
            store: self,
            key,
            sender,
            receiver: Some(receiver),
        }
    }

    /// Write-through persistence of one workflow run snapshot (Phase 5.0.1).
    /// Called by the orchestrator after every driver state transition so a
    /// crash/restart can detect and offer to resume an in-flight run.
    /// Replaces the row wholesale on every call — single-writer internal
    /// bookkeeping, not a user-facing edit, so there is no revision/
    /// optimistic-concurrency dance here (unlike pins/notes). Resuming a
    /// previously-abandoned run_id clears its `error` marker back to NULL,
    /// same as a brand new run.
    ///
    /// Stage A-6: when — and only when — the snapshot being written is a
    /// TERMINAL one, the same transaction also prunes this project's old
    /// finished runs (`prune_terminal_workflow_runs`). The gate is a string
    /// match in Rust, before any SQL, and that is the whole point: the
    /// orchestrator's 200ms driver calls this on every tick a live run
    /// changes, and every one of those snapshots is `'running'`, so the hot
    /// path pays one `matches!`-equivalent and nothing else. A run reaches a
    /// terminal state once, and `advance_all` stops ticking it immediately
    /// after, so the `count(*)` runs about once per finished run.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_workflow_run(
        &self,
        project: &Path,
        run_id: &str,
        name: &str,
        state: &str,
        started_at_ms: i64,
        ended_at_ms: Option<i64>,
        steps_json: &str,
    ) -> Result<(), String> {
        let project = project_id(project)?;
        let mut connection = self.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        transaction
            .execute(
                "INSERT INTO workflow_runs(
                   run_id, project_dir, name, state, started_at_ms, ended_at_ms,
                   steps_json, error
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)
                 ON CONFLICT(run_id) DO UPDATE SET
                   project_dir = excluded.project_dir,
                   name = excluded.name,
                   state = excluded.state,
                   started_at_ms = excluded.started_at_ms,
                   ended_at_ms = excluded.ended_at_ms,
                   steps_json = excluded.steps_json,
                   error = NULL",
                params![
                    run_id,
                    project,
                    name,
                    state,
                    started_at_ms,
                    ended_at_ms,
                    steps_json
                ],
            )
            .map_err(db_error)?;
        // Stage A-6, AFTER the row is in the transaction: the run that just
        // ended has to be inside the window it is measured against, otherwise
        // a project sitting exactly on the cap would delete the oldest row on
        // every completion whether or not it needed to.
        if is_terminal_workflow_state(state) {
            prune_terminal_workflow_runs(
                &transaction,
                &project,
                MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT,
            )?;
        }
        transaction.commit().map_err(db_error)?;
        Ok(())
    }

    /// Every run this project has left `state = 'running'` — a restart loses
    /// the in-memory `WorkflowRegistry`, so this is how `load_config` finds
    /// runs to offer resuming (Phase 5.0.1). Most-recently-started first.
    pub fn list_running_workflow_runs(
        &self,
        project: &Path,
    ) -> Result<Vec<WorkflowRunRow>, String> {
        let project = project_id(project)?;
        let connection = self.lock();
        let mut statement = connection
            .prepare(
                "SELECT run_id, name, state, started_at_ms, ended_at_ms, steps_json,
                        project_dir, error
                 FROM workflow_runs WHERE project_dir = ?1 AND state = 'running'
                 ORDER BY started_at_ms DESC",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map(params![project], workflow_run_from_row)
            .map_err(db_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
    }

    /// Discard a run left `running` from before a crash/restart instead of
    /// resuming it (Phase 5.0.1 "破棄"): marks it `cancelled` with an
    /// explanatory `error`, so `list_running_workflow_runs` — and therefore
    /// `load_config`'s resume prompt — never surfaces it again.
    ///
    /// Stage A-6: this is the OTHER way a run becomes terminal — it never
    /// goes through `upsert_workflow_run`, so it prunes here too, or an
    /// install whose runs are all discarded rather than finished would keep
    /// growing unbounded. Once per operator click on "discard", never on a
    /// driver tick.
    pub fn mark_workflow_abandoned(&self, project: &Path, run_id: &str) -> Result<(), String> {
        let project = project_id(project)?;
        let mut connection = self.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let changed = transaction
            .execute(
                "UPDATE workflow_runs SET state = 'cancelled',
                   error = 'abandoned after restart'
                 WHERE project_dir = ?1 AND run_id = ?2",
                params![project, run_id],
            )
            .map_err(db_error)?
            > 0;
        if !changed {
            return Err(format!("workflow run '{run_id}' not found"));
        }
        prune_terminal_workflow_runs(
            &transaction,
            &project,
            MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT,
        )?;
        transaction.commit().map_err(db_error)?;
        Ok(())
    }

    /// Stage A-6, v0.5.9 §2.4: apply the retention window to EVERY project in
    /// the table at once. Called exactly once, from `open`.
    ///
    /// The two existing entry points are both driven by a run ending, which
    /// means a project that has already stopped running workflows is out of
    /// their reach forever — including every project whose history was grown
    /// by a build that predates the cap. This is the only sweep that does not
    /// need a run to happen first.
    ///
    /// Reads the project list out of `workflow_runs` rather than from the
    /// config, and deliberately does NOT put it through `project_id`: the
    /// column already holds the canonicalised path this table was written
    /// with, and `project_id` would `canonicalize()` it again — failing, and
    /// so skipping the sweep, for exactly the abandoned projects (deleted or
    /// moved directories) whose rows are most likely to be the stale ones.
    ///
    /// One transaction for the whole sweep: it runs before any command handler
    /// can be invoked, so there is nothing to contend with, and a partial
    /// sweep is not a state worth being able to observe.
    pub fn prune_every_projects_terminal_workflow_runs(&self) -> Result<usize, String> {
        let mut connection = self.lock();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let projects: Vec<String> = {
            let [succeeded, failed, cancelled] = TERMINAL_WORKFLOW_STATES;
            let mut statement = transaction
                .prepare(
                    "SELECT DISTINCT project_dir FROM workflow_runs
                     WHERE state IN (?1, ?2, ?3)",
                )
                .map_err(db_error)?;
            let rows = statement
                .query_map(params![succeeded, failed, cancelled], |row| row.get(0))
                .map_err(db_error)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(db_error)?
        };
        for project in &projects {
            prune_terminal_workflow_runs(
                &transaction,
                project,
                MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT,
            )?;
        }
        transaction.commit().map_err(db_error)?;
        Ok(projects.len())
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn project_id(project: &Path) -> Result<String, String> {
    project
        .canonicalize()
        .map(|path| path.display().to_string())
        .map_err(|e| format!("cannot resolve Queen project {}: {e}", project.display()))
}

fn validated_required(label: &str, value: String, max: usize) -> Result<String, String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    validate_max(label, &value, max)?;
    Ok(value)
}

fn validate_max(label: &str, value: &str, max: usize) -> Result<(), String> {
    if value.len() > max {
        return Err(format!("{label} is too large (max {max} bytes)"));
    }
    Ok(())
}

fn validated_tags(tags: Vec<String>) -> Result<Vec<String>, String> {
    if tags.len() > MAX_TAGS {
        return Err(format!("too many note tags (max {MAX_TAGS})"));
    }
    tags.into_iter()
        .map(|tag| validated_required("note tag", tag, MAX_TAG_BYTES))
        .collect()
}

fn validated_mailbox(label: &str, value: String) -> Result<String, String> {
    let value = validated_required(label, value, MAX_MAILBOX_BYTES)?;
    if value.starts_with('#') {
        return Err(format!(
            "{label} must be a stable mailbox name, not a session #id"
        ));
    }
    Ok(value)
}

fn validate_note_id(id: i64) -> Result<(), String> {
    if id <= 0 {
        Err("note id must be a positive integer".to_string())
    } else {
        Ok(())
    }
}

fn validate_message_id(id: i64) -> Result<(), String> {
    if id <= 0 {
        Err("inbox message id must be a positive integer".to_string())
    } else {
        Ok(())
    }
}

fn validate_revision(revision: i64) -> Result<(), String> {
    if revision <= 0 {
        Err("expectedRevision must be a positive integer".to_string())
    } else {
        Ok(())
    }
}

fn enforce_limit(
    transaction: &rusqlite::Transaction<'_>,
    table: &str,
    project: &str,
    max: i64,
) -> Result<(), String> {
    let sql = format!("SELECT count(*) FROM {table} WHERE project_dir = ?1");
    let count: i64 = transaction
        .query_row(&sql, params![project], |row| row.get(0))
        .map_err(db_error)?;
    if count >= max {
        return Err(format!("Queen {table} limit reached (max {max})"));
    }
    Ok(())
}

/// Stage A-6: is this persisted `workflow_runs.state` a finished run?
/// Anything not on the allow-list — `'running'`, `'pending'`, or a value this
/// build has never heard of — counts as still-live and is out of retention's
/// reach entirely.
fn is_terminal_workflow_state(state: &str) -> bool {
    TERMINAL_WORKFLOW_STATES.contains(&state)
}

/// Stage A-6: keep at most `max` terminal `workflow_runs` rows for `project`,
/// newest first, and delete the rest.
///
/// This is retention by DELETE, and that is the one place it departs from
/// `enforce_limit` above: pins / notes / inbox are capped by REFUSING the
/// write, because each of those writes is a user asking for something and can
/// be answered with "limit reached". A `workflow_runs` write is not a request
/// — it is `orchestrator::persist_run` recording something that has already
/// happened, and whose `Err` it swallows. Refusing it would not stop the run;
/// it would silently drop the run's durable record and, with it, the ability
/// to resume it. So the old rows go, not the new one.
///
/// **What is never deleted.** Only states on `TERMINAL_WORKFLOW_STATES` are
/// counted or touched. A run still `'running'` is exactly what
/// `list_running_workflow_runs` — and therefore `load_config`'s "resume this
/// interrupted run?" banner (Phase 5.0.1) — exists to find, so deleting one
/// would silently make a crash unrecoverable. Non-terminal runs are also not
/// counted towards `max`, so no amount of finished history can push a live
/// run out, and no number of live runs can shrink the history window.
/// A run abandoned via `mark_workflow_abandoned` IS terminal
/// (`state = 'cancelled'`) and therefore prunable — the operator has already
/// declined to resume it, and its `error` marker only has to outlive the
/// resume prompt it was written for.
///
/// **Ordering** matches `orchestrator::evict_terminal` exactly, so the
/// in-memory and on-disk windows drop the same runs:
/// `COALESCE(ended_at_ms, started_at_ms) DESC, run_id DESC`. The `COALESCE`
/// is load-bearing rather than defensive — `spawn_workflow` can publish an
/// already-terminal run with `ended_at_ms` still `NULL` when every root fails
/// to spawn, and sorting those to the bottom would evict the newest runs
/// first. `run_id` breaks ties because `new_run_id` is a monotonic nanosecond
/// stamp in fixed-width hex, so its string order agrees with creation order.
///
/// **Cost.** The `count(*)` short-circuits the common case (under the cap)
/// before any sort, same shape as `enforce_limit` and as `evict_terminal`'s
/// early return. Both statements filter on `(project_dir, state)`, the
/// leading columns of the existing `workflow_runs_project_state` index, so
/// Stage A-6 needs no new index and therefore no `user_version` bump — the
/// numbering for 4 is still unclaimed between 5.6.0 and 6.0.0.
fn prune_terminal_workflow_runs(
    transaction: &rusqlite::Transaction<'_>,
    project: &str,
    max: i64,
) -> Result<(), String> {
    let [succeeded, failed, cancelled] = TERMINAL_WORKFLOW_STATES;
    let count: i64 = transaction
        .query_row(
            "SELECT count(*) FROM workflow_runs
             WHERE project_dir = ?1 AND state IN (?2, ?3, ?4)",
            params![project, succeeded, failed, cancelled],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    if count <= max {
        return Ok(());
    }
    transaction
        .execute(
            "DELETE FROM workflow_runs
             WHERE project_dir = ?1 AND state IN (?2, ?3, ?4)
               AND run_id NOT IN (
                 SELECT run_id FROM workflow_runs
                 WHERE project_dir = ?1 AND state IN (?2, ?3, ?4)
                 ORDER BY COALESCE(ended_at_ms, started_at_ms) DESC, run_id DESC
                 LIMIT ?5
               )",
            params![project, succeeded, failed, cancelled, max],
        )
        .map_err(db_error)?;
    Ok(())
}

fn pin_from_row(row: &Row<'_>) -> rusqlite::Result<Pin> {
    Ok(Pin {
        key: row.get(0)?,
        value: row.get(1)?,
        revision: row.get(2)?,
        created_at_ms: row.get(3)?,
        updated_at_ms: row.get(4)?,
    })
}

fn note_from_row(row: &Row<'_>) -> rusqlite::Result<Note> {
    let tags_json: String = row.get(3)?;
    let tags = serde_json::from_str(&tags_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
    })?;
    Ok(Note {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        tags,
        revision: row.get(4)?,
        created_at_ms: row.get(5)?,
        updated_at_ms: row.get(6)?,
    })
}

fn inbox_from_row(row: &Row<'_>) -> rusqlite::Result<InboxMessage> {
    Ok(InboxMessage {
        id: row.get(0)?,
        sender: row.get(1)?,
        recipient: row.get(2)?,
        subject: row.get(3)?,
        body: row.get(4)?,
        in_reply_to_id: row.get(5)?,
        root_message_id: row.get(6)?,
        acknowledged_at_ms: row.get(7)?,
        created_at_ms: row.get(8)?,
    })
}

fn workflow_run_from_row(row: &Row<'_>) -> rusqlite::Result<WorkflowRunRow> {
    Ok(WorkflowRunRow {
        run_id: row.get(0)?,
        name: row.get(1)?,
        state: row.get(2)?,
        started_at_ms: row.get(3)?,
        ended_at_ms: row.get(4)?,
        steps_json: row.get(5)?,
        project_dir: row.get(6)?,
        error: row.get(7)?,
    })
}

fn get_note_from(connection: &Connection, project: &str, id: i64) -> Result<Option<Note>, String> {
    connection
        .query_row(
            "SELECT id, title, body, tags_json, revision, created_at_ms, updated_at_ms
             FROM notes WHERE project_dir = ?1 AND id = ?2",
            params![project, id],
            note_from_row,
        )
        .optional()
        .map_err(db_error)
}

fn get_inbox_from(
    connection: &Connection,
    project: &str,
    id: i64,
) -> Result<Option<InboxMessage>, String> {
    connection
        .query_row(
            "SELECT id, sender, recipient, subject, body, in_reply_to_id,
                    root_message_id, acknowledged_at_ms, created_at_ms
             FROM inbox_messages WHERE project_dir = ?1 AND id = ?2",
            params![project, id],
            inbox_from_row,
        )
        .optional()
        .map_err(db_error)
}

fn db_error(error: rusqlite::Error) -> String {
    format!("Queen database error: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};

    static NEXT_TEST: AtomicU64 = AtomicU64::new(1);

    fn projects() -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "ptygrid-queen-store-{}-{}",
            std::process::id(),
            NEXT_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        let one = root.join("one");
        let two = root.join("two");
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        (root, one, two)
    }

    fn wait_options(after_id: i64, timeout: Duration) -> InboxWaitOptions {
        InboxWaitOptions {
            mailbox: "codex".to_string(),
            after_id,
            include_acknowledged: false,
            limit: 50,
            timeout,
        }
    }

    #[test]
    fn pins_upsert_delete_persist_and_are_project_scoped() {
        let (root, one, two) = projects();
        let database = root.join("data/queen.sqlite3");
        {
            let store = QueenStore::open(&database).unwrap();
            let created = store
                .set_pin(&one, " objective ".to_string(), "ship".to_string(), None)
                .unwrap();
            assert_eq!(created.key, "objective");
            let updated = store
                .set_pin(
                    &one,
                    "objective".to_string(),
                    "verify".to_string(),
                    Some(created.revision),
                )
                .unwrap();
            assert_eq!(updated.created_at_ms, created.created_at_ms);
            assert_eq!(updated.value, "verify");
            assert!(store
                .set_pin(
                    &one,
                    "objective".to_string(),
                    "stale overwrite".to_string(),
                    Some(created.revision),
                )
                .unwrap_err()
                .contains("conflict"));
            assert_eq!(store.list_pins(&one).unwrap()[0].value, "verify");
            store
                .set_pin(&two, "objective".to_string(), "other".to_string(), None)
                .unwrap();
        }
        {
            let store = QueenStore::open(&database).unwrap();
            assert_eq!(store.list_pins(&one).unwrap()[0].value, "verify");
            assert_eq!(store.list_pins(&two).unwrap()[0].value, "other");
            let pin = &store.list_pins(&one).unwrap()[0];
            store
                .delete_pin(&one, "objective".to_string(), pin.revision)
                .unwrap();
            assert!(store
                .delete_pin(&one, "objective".to_string(), pin.revision)
                .is_err());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn simultaneous_pin_updates_allow_exactly_one_writer() {
        let (root, one, _) = projects();
        let store = Arc::new(QueenStore::open_in_memory().unwrap());
        let created = store
            .set_pin(&one, "owner".to_string(), "unassigned".to_string(), None)
            .unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let mut writers = Vec::new();

        for value in ["codex", "claude"] {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            let project = one.clone();
            let expected_revision = created.revision;
            writers.push(std::thread::spawn(move || {
                barrier.wait();
                store.set_pin(
                    &project,
                    "owner".to_string(),
                    value.to_string(),
                    Some(expected_revision),
                )
            }));
        }

        barrier.wait();
        let results: Vec<_> = writers
            .into_iter()
            .map(|writer| writer.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(error) if error.contains("conflict")))
                .count(),
            1
        );
        let latest = store.list_pins(&one).unwrap().pop().unwrap();
        assert_eq!(latest.revision, created.revision + 1);
        assert!(latest.value == "codex" || latest.value == "claude");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn notes_support_crud_search_and_project_isolation() {
        let (root, one, two) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        let created = store
            .create_note(
                &one,
                "Decision".to_string(),
                "Use SQLite transactions".to_string(),
                vec!["architecture".to_string()],
            )
            .unwrap();
        store
            .create_note(
                &two,
                "Hidden".to_string(),
                "other project".to_string(),
                Vec::new(),
            )
            .unwrap();
        assert_eq!(store.list_notes(&one, None, 50).unwrap().len(), 1);
        assert_eq!(
            store
                .list_notes(&one, Some("sqlite".to_string()), 50)
                .unwrap()[0]
                .id,
            created.id
        );
        assert!(store
            .list_notes(&one, Some("missing".to_string()), 50)
            .unwrap()
            .is_empty());

        let updated = store
            .update_note(
                &one,
                created.id,
                created.revision,
                Some("Final decision".to_string()),
                None,
                Some(vec!["done".to_string()]),
            )
            .unwrap();
        assert_eq!(updated.title, "Final decision");
        assert_eq!(updated.body, created.body);
        assert_eq!(updated.tags, vec!["done"]);
        assert!(store
            .update_note(
                &one,
                created.id,
                created.revision,
                None,
                Some("stale overwrite".to_string()),
                None,
            )
            .unwrap_err()
            .contains("conflict"));
        assert_eq!(
            store.get_note(&one, created.id).unwrap().unwrap().body,
            created.body
        );
        assert!(store.get_note(&two, created.id).unwrap().is_none());
        store
            .delete_note(&one, created.id, updated.revision)
            .unwrap();
        assert!(store.get_note(&one, created.id).unwrap().is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn inbox_is_project_scoped_and_acknowledgement_is_idempotent() {
        let (root, one, two) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        let initial_generation = *store.inbox_generation.borrow();
        let message = store
            .send_inbox(
                &one,
                "claude-impl".to_string(),
                "codex-review".to_string(),
                "Review session storage".to_string(),
                "Please inspect the migration.".to_string(),
            )
            .unwrap();
        let sent_generation = *store.inbox_generation.borrow();
        assert_eq!(sent_generation, initial_generation + 1);
        assert_eq!(message.root_message_id, message.id);
        assert_eq!(message.in_reply_to_id, None);
        assert_eq!(message.acknowledged_at_ms, None);
        assert_eq!(
            store
                .list_inbox(&one, "codex-review".to_string(), 0, false, 50)
                .unwrap(),
            vec![message.clone()]
        );
        assert!(store
            .list_inbox(&two, "codex-review".to_string(), 0, true, 50)
            .unwrap()
            .is_empty());
        assert!(store
            .ack_inbox(&one, message.id, "wrong-mailbox".to_string())
            .unwrap_err()
            .contains("belongs to recipient"));

        let acknowledged = store
            .ack_inbox(&one, message.id, "codex-review".to_string())
            .unwrap();
        assert_eq!(*store.inbox_generation.borrow(), sent_generation);
        assert!(acknowledged.acknowledged_at_ms.is_some());
        let repeated = store
            .ack_inbox(&one, message.id, "codex-review".to_string())
            .unwrap();
        assert_eq!(repeated, acknowledged);
        assert!(store
            .list_inbox(&one, "codex-review".to_string(), 0, false, 50)
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .list_inbox(&one, "codex-review".to_string(), 0, true, 50)
                .unwrap(),
            vec![acknowledged]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// Stage A-5. Three properties in one, because they are the three the
    /// orchestrator's cancel/abandon sweep leans on:
    ///
    /// 1. the selector is the SENDER, so one sender's leftovers can be closed
    ///    without touching another's — that is what keeps a cancelled run from
    ///    acknowledging a live sibling run's kickoff;
    /// 2. it is project-scoped like every other store method;
    /// 3. it never re-stamps an already-acknowledged message, and reports 0
    ///    the second time, so a repeat sweep is a genuine no-op.
    #[test]
    fn acking_by_sender_closes_only_that_senders_unacknowledged_messages() {
        let (root, one, two) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        let send = |project: &Path, sender: &str, recipient: &str| {
            store
                .send_inbox(
                    project,
                    sender.to_string(),
                    recipient.to_string(),
                    "kickoff".to_string(),
                    format!("work for {recipient}"),
                )
                .unwrap()
        };
        send(&one, "queen:workflow/demo/run-a", "coder");
        send(&one, "queen:workflow/demo/run-a", "reviewer");
        let sibling = send(&one, "queen:workflow/demo/run-b", "coder");
        send(&two, "queen:workflow/demo/run-a", "coder");
        // Already answered, so already acknowledged: the sweep must leave its
        // original timestamp alone rather than move it to "now".
        let answered = send(&one, "queen:workflow/demo/run-a", "writer");
        store
            .reply_inbox(&one, answered.id, "writer".to_string(), "done".to_string())
            .unwrap();
        let answered_before = store
            .list_inbox(&one, "writer".to_string(), 0, true, 50)
            .unwrap();
        assert!(answered_before[0].acknowledged_at_ms.is_some());

        assert_eq!(
            store
                .ack_inbox_from_sender(&one, "queen:workflow/demo/run-a".to_string())
                .unwrap(),
            2,
            "only the two unacknowledged messages of run-a are counted"
        );
        assert!(store
            .list_inbox(&one, "reviewer".to_string(), 0, false, 50)
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .list_inbox(&one, "writer".to_string(), 0, true, 50)
                .unwrap(),
            answered_before,
            "an already-acknowledged message keeps its original stamp"
        );
        assert_eq!(
            store
                .list_inbox(&one, "coder".to_string(), 0, false, 50)
                .unwrap(),
            vec![sibling],
            "another run's kickoff is a different sender and is untouched"
        );
        assert_eq!(
            store
                .list_inbox(&two, "coder".to_string(), 0, false, 50)
                .unwrap()
                .len(),
            1,
            "same sender in another project is untouched"
        );
        assert_eq!(
            store
                .ack_inbox_from_sender(&one, "queen:workflow/demo/run-a".to_string())
                .unwrap(),
            0,
            "sweeping twice closes nothing the second time"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn replies_reverse_mailboxes_preserve_thread_and_acknowledge_original() {
        let (root, one, _) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        let root_message = store
            .send_inbox(
                &one,
                "claude-impl".to_string(),
                "codex-review".to_string(),
                "Review request".to_string(),
                "Ready for review".to_string(),
            )
            .unwrap();
        assert!(store
            .reply_inbox(
                &one,
                root_message.id,
                "claude-impl".to_string(),
                "spoofed".to_string(),
            )
            .unwrap_err()
            .contains("only recipient"));

        let reply = store
            .reply_inbox(
                &one,
                root_message.id,
                "codex-review".to_string(),
                "Looks good".to_string(),
            )
            .unwrap();
        assert_eq!(reply.sender, "codex-review");
        assert_eq!(reply.recipient, "claude-impl");
        assert_eq!(reply.subject, root_message.subject);
        assert_eq!(reply.in_reply_to_id, Some(root_message.id));
        assert_eq!(reply.root_message_id, root_message.id);
        assert!(store
            .list_inbox(&one, "codex-review".to_string(), 0, false, 50)
            .unwrap()
            .is_empty());

        let second_reply = store
            .reply_inbox(
                &one,
                reply.id,
                "claude-impl".to_string(),
                "Thanks".to_string(),
            )
            .unwrap();
        assert_eq!(second_reply.in_reply_to_id, Some(reply.id));
        assert_eq!(second_reply.root_message_id, root_message.id);
        let claude_inbox = store
            .list_inbox(&one, "claude-impl".to_string(), 0, true, 50)
            .unwrap();
        assert_eq!(claude_inbox.len(), 1);
        assert_eq!(claude_inbox[0].id, reply.id);
        assert!(claude_inbox[0].acknowledged_at_ms.is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn await_inbox_returns_immediately_wakes_times_out_and_cancels() {
        let (root, one, _) = projects();
        let store = Arc::new(QueenStore::open_in_memory().unwrap());
        let existing = store
            .send_inbox(
                &one,
                "claude".to_string(),
                "codex".to_string(),
                "existing".to_string(),
                "ready".to_string(),
            )
            .unwrap();
        let pre_cancelled = CancellationToken::new();
        pre_cancelled.cancel();
        assert_eq!(
            store
                .await_inbox(&one, wait_options(0, Duration::from_secs(1)), pre_cancelled,)
                .await
                .unwrap(),
            InboxWait::Cancelled
        );
        let immediate = store
            .await_inbox(
                &one,
                wait_options(0, Duration::from_secs(1)),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(immediate, InboxWait::Messages(vec![existing.clone()]));

        let waiting_store = Arc::clone(&store);
        let waiting_project = one.clone();
        let waiting = tokio::spawn(async move {
            waiting_store
                .await_inbox(
                    &waiting_project,
                    wait_options(existing.id, Duration::from_secs(1)),
                    CancellationToken::new(),
                )
                .await
        });
        tokio::task::yield_now().await;
        let arrived = store
            .send_inbox(
                &one,
                "claude".to_string(),
                "codex".to_string(),
                "new".to_string(),
                "wake up".to_string(),
            )
            .unwrap();
        assert_eq!(
            waiting.await.unwrap().unwrap(),
            InboxWait::Messages(vec![arrived.clone()])
        );

        let timed_out = store
            .await_inbox(
                &one,
                wait_options(arrived.id, Duration::from_millis(5)),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(timed_out, InboxWait::TimedOut);

        let cancellation = CancellationToken::new();
        let cancel_handle = cancellation.clone();
        let cancelling_store = Arc::clone(&store);
        let cancelling_project = one.clone();
        let cancelling = tokio::spawn(async move {
            cancelling_store
                .await_inbox(
                    &cancelling_project,
                    wait_options(arrived.id, Duration::from_secs(1)),
                    cancellation,
                )
                .await
        });
        tokio::task::yield_now().await;
        cancel_handle.cancel();
        assert_eq!(cancelling.await.unwrap().unwrap(), InboxWait::Cancelled);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn await_inbox_is_not_woken_by_another_mailbox() {
        let (root, one, _) = projects();
        let store = Arc::new(QueenStore::open_in_memory().unwrap());
        let waiting_store = Arc::clone(&store);
        let waiting_project = one.clone();
        let waiting = tokio::spawn(async move {
            waiting_store
                .await_inbox(
                    &waiting_project,
                    wait_options(0, Duration::from_millis(150)),
                    CancellationToken::new(),
                )
                .await
        });
        tokio::task::yield_now().await;
        assert_eq!(store.mailbox_waiter_count(), 1);
        assert_eq!(store.mailbox_generation(&one, "codex"), Some(0));
        for subject in ["first", "second", "third"] {
            store
                .send_inbox(
                    &one,
                    "claude".to_string(),
                    "someone-else".to_string(),
                    subject.to_string(),
                    "not addressed to codex".to_string(),
                )
                .unwrap();
        }
        // Traffic on another mailbox neither registers a channel of its own
        // nor disturbs the waiter on "codex": its generation never moves, so
        // the waiter is never woken and sleeps out its full deadline.
        assert_eq!(store.mailbox_waiter_count(), 1);
        assert_eq!(store.mailbox_generation(&one, "codex"), Some(0));
        assert_eq!(*store.inbox_generation.borrow(), 3);
        assert_eq!(waiting.await.unwrap().unwrap(), InboxWait::TimedOut);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mailbox_watch_entry_is_dropped_when_the_last_waiter_leaves() {
        let (root, one, _) = projects();
        let store = Arc::new(QueenStore::open_in_memory().unwrap());
        assert_eq!(store.mailbox_waiter_count(), 0);
        let mut waiters = Vec::new();
        for _ in 0..2 {
            let waiting_store = Arc::clone(&store);
            let waiting_project = one.clone();
            waiters.push(tokio::spawn(async move {
                waiting_store
                    .await_inbox(
                        &waiting_project,
                        wait_options(0, Duration::from_millis(50)),
                        CancellationToken::new(),
                    )
                    .await
            }));
        }
        tokio::task::yield_now().await;
        // Both waiters share the single entry for "codex".
        assert_eq!(store.mailbox_waiter_count(), 1);
        for waiter in waiters {
            assert_eq!(waiter.await.unwrap().unwrap(), InboxWait::TimedOut);
        }
        assert_eq!(store.mailbox_waiter_count(), 0);

        // A call that returns without ever blocking cleans up just the same.
        store
            .send_inbox(
                &one,
                "claude".to_string(),
                "codex".to_string(),
                "immediate".to_string(),
                "already here".to_string(),
            )
            .unwrap();
        assert!(matches!(
            store
                .await_inbox(
                    &one,
                    wait_options(0, Duration::from_secs(1)),
                    CancellationToken::new(),
                )
                .await
                .unwrap(),
            InboxWait::Messages(_)
        ));
        assert_eq!(store.mailbox_waiter_count(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn subscribe_mailbox_after_a_sibling_drop_still_wakes_on_notify() {
        // Regression test for the lost-wakeup race on `subscribe_mailbox`:
        // subscribing must be atomic with the map lookup, or a sibling
        // waiter's `Drop` can evict the channel before the new subscriber
        // ever registers on it, orphaning it until timeout.
        let (root, one, _) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        let key = (project_id(&one).unwrap(), "codex".to_string());

        // Waiter A subscribes first, creating the shared channel.
        let sub_a = store.subscribe_mailbox(key.clone());
        assert_eq!(store.mailbox_waiter_count(), 1);

        // Waiter B subscribes to that same still-live channel...
        let mut sub_b = store.subscribe_mailbox(key.clone());
        // ...only then does A leave. This is exactly the ordering the fix
        // must preserve: by the time A's `Drop` checks `receiver_count()`,
        // B must already be registered on the same sender, so the entry is
        // *not* removed out from under B.
        drop(sub_a);
        assert_eq!(
            store.mailbox_waiter_count(),
            1,
            "B's subscription must keep the mailbox entry alive"
        );

        store
            .send_inbox(
                &one,
                "claude".to_string(),
                "codex".to_string(),
                "subject".to_string(),
                "hello".to_string(),
            )
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), sub_b.receiver_mut().changed())
            .await
            .expect("B must be woken by notify_inbox instead of being orphaned")
            .expect("watch channel must still be open");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn two_waiters_on_one_mailbox_are_both_woken() {
        let (root, one, _) = projects();
        let store = Arc::new(QueenStore::open_in_memory().unwrap());
        let mut waiters = Vec::new();
        for _ in 0..2 {
            let waiting_store = Arc::clone(&store);
            let waiting_project = one.clone();
            waiters.push(tokio::spawn(async move {
                waiting_store
                    .await_inbox(
                        &waiting_project,
                        wait_options(0, Duration::from_secs(5)),
                        CancellationToken::new(),
                    )
                    .await
            }));
        }
        tokio::task::yield_now().await;
        assert_eq!(store.mailbox_waiter_count(), 1);
        let arrived = store
            .send_inbox(
                &one,
                "claude".to_string(),
                "codex".to_string(),
                "broadcast".to_string(),
                "wake everyone".to_string(),
            )
            .unwrap();
        for waiter in waiters {
            assert_eq!(
                waiter.await.unwrap().unwrap(),
                InboxWait::Messages(vec![arrived.clone()])
            );
        }
        assert_eq!(store.mailbox_waiter_count(), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_invalid_mutations_without_partial_writes() {
        let (root, one, _) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        assert!(store
            .set_pin(&one, " ".to_string(), "value".to_string(), None)
            .is_err());
        assert!(store.list_pins(&one).unwrap().is_empty());
        let note = store
            .create_note(&one, "valid".to_string(), String::new(), Vec::new())
            .unwrap();
        assert!(store
            .update_note(&one, note.id, note.revision, None, None, None)
            .is_err());
        assert_eq!(store.get_note(&one, note.id).unwrap(), Some(note));
        assert!(store
            .send_inbox(
                &one,
                "#3".to_string(),
                "codex-review".to_string(),
                "subject".to_string(),
                "body".to_string(),
            )
            .unwrap_err()
            .contains("stable mailbox"));
        assert!(store
            .list_inbox(&one, "codex-review".to_string(), 0, true, 50)
            .unwrap()
            .is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_version_one_without_losing_existing_data() {
        let (root, one, _) = projects();
        let database = root.join("data/queen.sqlite3");
        std::fs::create_dir_all(database.parent().unwrap()).unwrap();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE pins (
                   project_dir TEXT NOT NULL,
                   pin_key TEXT NOT NULL,
                   value TEXT NOT NULL,
                   revision INTEGER NOT NULL,
                   created_at_ms INTEGER NOT NULL,
                   updated_at_ms INTEGER NOT NULL,
                   PRIMARY KEY (project_dir, pin_key)
                 );
                 CREATE TABLE notes (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   project_dir TEXT NOT NULL,
                   title TEXT NOT NULL,
                   body TEXT NOT NULL,
                   tags_json TEXT NOT NULL,
                   revision INTEGER NOT NULL,
                   created_at_ms INTEGER NOT NULL,
                   updated_at_ms INTEGER NOT NULL
                 );
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        let project = project_id(&one).unwrap();
        connection
            .execute(
                "INSERT INTO pins VALUES (?1, 'existing', 'kept', 1, 1, 1)",
                params![project],
            )
            .unwrap();
        drop(connection);

        let store = QueenStore::open(&database).unwrap();
        assert_eq!(store.list_pins(&one).unwrap()[0].value, "kept");
        assert!(store
            .send_inbox(
                &one,
                "claude".to_string(),
                "codex".to_string(),
                "migrated".to_string(),
                "ready".to_string(),
            )
            .is_ok());
        let version: i64 = store
            .lock()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        // Phase 5.0.1: v1 now migrates all the way to v3 (workflow_runs
        // added), not just to v2.
        assert_eq!(version, 3);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn migrates_version_one_when_inbox_table_already_exists() {
        // L12a: a partially-migrated db (user_version still 1 but the v2
        // inbox table/indexes already present) must migrate idempotently
        // instead of hard-failing `open` with "table already exists".
        let (root, one, _) = projects();
        let database = root.join("data/queen.sqlite3");
        std::fs::create_dir_all(database.parent().unwrap()).unwrap();
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE pins (
                   project_dir TEXT NOT NULL,
                   pin_key TEXT NOT NULL,
                   value TEXT NOT NULL,
                   revision INTEGER NOT NULL,
                   created_at_ms INTEGER NOT NULL,
                   updated_at_ms INTEGER NOT NULL,
                   PRIMARY KEY (project_dir, pin_key)
                 );
                 CREATE TABLE notes (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   project_dir TEXT NOT NULL,
                   title TEXT NOT NULL,
                   body TEXT NOT NULL,
                   tags_json TEXT NOT NULL,
                   revision INTEGER NOT NULL,
                   created_at_ms INTEGER NOT NULL,
                   updated_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE inbox_messages (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   project_dir TEXT NOT NULL,
                   sender TEXT NOT NULL,
                   recipient TEXT NOT NULL,
                   subject TEXT NOT NULL,
                   body TEXT NOT NULL,
                   in_reply_to_id INTEGER,
                   root_message_id INTEGER,
                   acknowledged_at_ms INTEGER,
                   created_at_ms INTEGER NOT NULL,
                   FOREIGN KEY (in_reply_to_id) REFERENCES inbox_messages(id),
                   FOREIGN KEY (root_message_id) REFERENCES inbox_messages(id)
                 );
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        drop(connection);

        let store = QueenStore::open(&database).unwrap();
        assert!(store
            .send_inbox(
                &one,
                "claude".to_string(),
                "codex".to_string(),
                "migrated".to_string(),
                "ready".to_string(),
            )
            .is_ok());
        let version: i64 = store
            .lock()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        // Phase 5.0.1: v1 now migrates all the way to v3 (workflow_runs
        // added), not just to v2.
        assert_eq!(version, 3);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_unknown_database_versions() {
        let (root, _, _) = projects();
        let database = root.join("data/queen.sqlite3");
        std::fs::create_dir_all(database.parent().unwrap()).unwrap();
        let connection = Connection::open(&database).unwrap();
        connection.pragma_update(None, "user_version", 4).unwrap();
        drop(connection);
        let error = QueenStore::open(&database).err().unwrap();
        assert!(error.contains("unsupported Queen database version"));
        let _ = std::fs::remove_dir_all(root);
    }

    // ------------------------------------------------------------------
    // Phase 5.0.1: workflow_runs (migration + CRUD)
    // ------------------------------------------------------------------

    #[test]
    fn migrates_version_two_to_three_and_workflow_runs_is_idempotent() {
        let (root, one, _) = projects();
        let database = root.join("data/queen.sqlite3");
        std::fs::create_dir_all(database.parent().unwrap()).unwrap();
        {
            // A pre-5.0.1 (v2) database: pins/notes/inbox_messages present,
            // no workflow_runs table yet.
            let connection = Connection::open(&database).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE pins (
                       project_dir TEXT NOT NULL,
                       pin_key TEXT NOT NULL,
                       value TEXT NOT NULL,
                       revision INTEGER NOT NULL,
                       created_at_ms INTEGER NOT NULL,
                       updated_at_ms INTEGER NOT NULL,
                       PRIMARY KEY (project_dir, pin_key)
                     );
                     CREATE TABLE notes (
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       project_dir TEXT NOT NULL,
                       title TEXT NOT NULL,
                       body TEXT NOT NULL,
                       tags_json TEXT NOT NULL,
                       revision INTEGER NOT NULL,
                       created_at_ms INTEGER NOT NULL,
                       updated_at_ms INTEGER NOT NULL
                     );
                     CREATE TABLE inbox_messages (
                       id INTEGER PRIMARY KEY AUTOINCREMENT,
                       project_dir TEXT NOT NULL,
                       sender TEXT NOT NULL,
                       recipient TEXT NOT NULL,
                       subject TEXT NOT NULL,
                       body TEXT NOT NULL,
                       in_reply_to_id INTEGER,
                       root_message_id INTEGER,
                       acknowledged_at_ms INTEGER,
                       created_at_ms INTEGER NOT NULL
                     );
                     PRAGMA user_version = 2;",
                )
                .unwrap();
        }

        let store = QueenStore::open(&database).unwrap();
        let version: i64 = store
            .lock()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 3);
        store
            .upsert_workflow_run(&one, "run-1", "demo", "running", 1_000, None, "[]")
            .unwrap();
        assert_eq!(store.list_running_workflow_runs(&one).unwrap().len(), 1);
        drop(store);

        // Re-opening an already-migrated (v3) database is idempotent and
        // keeps the row written above.
        let reopened = QueenStore::open(&database).unwrap();
        assert_eq!(reopened.list_running_workflow_runs(&one).unwrap().len(), 1);
        let version: i64 = reopened
            .lock()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 3);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workflow_runs_upsert_list_running_and_abandon_round_trip() {
        let (root, one, two) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        store
            .upsert_workflow_run(&one, "run-a", "demo", "running", 100, None, "[]")
            .unwrap();
        store
            .upsert_workflow_run(&one, "run-b", "other", "succeeded", 50, Some(60), "[]")
            .unwrap();
        store
            .upsert_workflow_run(&two, "run-c", "demo", "running", 10, None, "[]")
            .unwrap();

        let running_one = store.list_running_workflow_runs(&one).unwrap();
        assert_eq!(
            running_one.len(),
            1,
            "only the 'running' row for project one, project-scoped"
        );
        assert_eq!(running_one[0].run_id, "run-a");
        assert!(store
            .list_running_workflow_runs(&two)
            .unwrap()
            .iter()
            .any(|r| r.run_id == "run-c"));

        // Upsert on the same run_id replaces the row in place (write-through
        // semantics, not append-only).
        store
            .upsert_workflow_run(
                &one,
                "run-a",
                "demo",
                "running",
                100,
                None,
                "[{\"stepId\":\"first\"}]",
            )
            .unwrap();
        let refreshed = store.list_running_workflow_runs(&one).unwrap();
        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].steps_json, "[{\"stepId\":\"first\"}]");

        store.mark_workflow_abandoned(&one, "run-a").unwrap();
        assert!(store.list_running_workflow_runs(&one).unwrap().is_empty());
        assert!(store
            .mark_workflow_abandoned(&one, "does-not-exist")
            .unwrap_err()
            .contains("not found"));
        let _ = std::fs::remove_dir_all(root);
    }

    // ------------------------------------------------------------------
    // Stage A-6: workflow_runs retention
    // ------------------------------------------------------------------

    /// Every `workflow_runs` row this project still has, ordered the way
    /// retention orders them: newest kept first.
    fn stored_run_ids(store: &QueenStore, project: &Path) -> Vec<String> {
        let project = project_id(project).unwrap();
        let connection = store.lock();
        let mut statement = connection
            .prepare(
                "SELECT run_id FROM workflow_runs WHERE project_dir = ?1
                 ORDER BY COALESCE(ended_at_ms, started_at_ms) DESC, run_id DESC",
            )
            .unwrap();
        let ids = statement
            .query_map(params![project], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        ids
    }

    /// `count` finished runs, oldest first, so run `n` is always newer than
    /// run `n - 1` under retention's ordering key.
    fn store_finished_runs(store: &QueenStore, project: &Path, count: i64) {
        for n in 0..count {
            let ended = 1_000 + n;
            store
                .upsert_workflow_run(
                    project,
                    &format!("done-{n:05}"),
                    "demo",
                    "succeeded",
                    ended - 1,
                    Some(ended),
                    "[]",
                )
                .unwrap();
        }
    }

    #[test]
    fn finished_workflow_runs_past_the_cap_lose_the_oldest_rows_first() {
        let (root, one, _) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        let overshoot = 5;
        store_finished_runs(
            &store,
            &one,
            MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT + overshoot,
        );

        let kept = stored_run_ids(&store, &one);
        assert_eq!(
            kept.len() as i64,
            MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT,
            "retention trims back to the cap, it does not merely stop growing"
        );
        // The `overshoot` oldest runs are the ones gone, and the newest run —
        // the one whose own write triggered the prune — is still there.
        assert_eq!(
            kept.first().map(String::as_str),
            Some(
                format!("done-{:05}", MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT + overshoot - 1)
                    .as_str()
            )
        );
        assert_eq!(
            kept.last().map(String::as_str),
            Some(format!("done-{overshoot:05}").as_str())
        );
        for n in 0..overshoot {
            assert!(
                !kept.contains(&format!("done-{n:05}")),
                "run {n} is older than the cap allows and should be gone"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_unfinished_workflow_run_survives_any_amount_of_history_written_after_it() {
        let (root, one, _) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        // Deliberately the OLDEST row in the project: under a naive
        // "keep the newest N rows" rule this is the first thing to go, and
        // losing it is losing the resume banner for a crashed run.
        store
            .upsert_workflow_run(&one, "still-going", "demo", "running", 1, None, "[]")
            .unwrap();
        store_finished_runs(&store, &one, MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT + 50);

        let resumable = store.list_running_workflow_runs(&one).unwrap();
        assert_eq!(
            resumable.len(),
            1,
            "a run that never reached a terminal state is not retention's business"
        );
        assert_eq!(resumable[0].run_id, "still-going");
        // ...and it does not eat into the history window either: the cap
        // counts finished runs only.
        assert_eq!(
            stored_run_ids(&store, &one).len() as i64,
            MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT + 1
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_workflow_run_state_this_build_does_not_know_is_never_pruned() {
        let (root, one, _) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        // The allow-list claim (CONTRACT.md 続報17 (3)) in the only form that
        // can actually fail: a state that is on neither list. Every other
        // retention test uses 'running' or 'succeeded', so rewriting
        // `is_terminal_workflow_state` as "anything that is not 'running'"
        // would keep all of them green while a future schema's — or a hand
        // edited — row started being deleted. It is also the OLDEST row here,
        // so the ordering key puts it first in line to go.
        store
            .upsert_workflow_run(&one, "from-the-future", "demo", "weird", 1, Some(2), "[]")
            .unwrap();
        assert!(!is_terminal_workflow_state("weird"));
        store_finished_runs(&store, &one, MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT + 20);

        let kept = stored_run_ids(&store, &one);
        assert!(
            kept.contains(&"from-the-future".to_string()),
            "retention must err towards keeping: an unrecognised state counts \
             as still-live and is out of its reach entirely"
        );
        assert_eq!(
            kept.len() as i64,
            MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT + 1,
            "...and it does not eat into the window either — only the three \
             terminal states are counted against the cap"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workflow_run_retention_deletes_nothing_while_the_project_is_under_its_cap() {
        let (root, one, _) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        store_finished_runs(&store, &one, MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT);

        assert_eq!(
            stored_run_ids(&store, &one).len() as i64,
            MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT,
            "sitting exactly on the cap is not over it"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workflow_run_retention_gives_every_project_its_own_window() {
        let (root, one, two) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        store
            .upsert_workflow_run(&two, "quiet-project", "demo", "succeeded", 1, Some(2), "[]")
            .unwrap();
        store_finished_runs(&store, &one, MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT + 20);

        assert_eq!(
            stored_run_ids(&store, &two),
            vec!["quiet-project".to_string()],
            "a busy project must not evict a quiet one's single run"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn abandoning_a_run_prunes_the_history_it_has_just_joined() {
        let (root, one, _) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        store_finished_runs(&store, &one, MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT);
        // A crash-interrupted run the operator discards: it never passes
        // through `upsert_workflow_run` again, so this is the only chance to
        // notice that the project is now one over the cap.
        store
            .upsert_workflow_run(&one, "interrupted", "demo", "running", 9_000, None, "[]")
            .unwrap();
        store.mark_workflow_abandoned(&one, "interrupted").unwrap();

        let kept = stored_run_ids(&store, &one);
        assert_eq!(kept.len() as i64, MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT);
        assert!(
            kept.contains(&"interrupted".to_string()),
            "the abandoned run is the newest one, so it is what stays"
        );
        assert!(
            !kept.contains(&"done-00000".to_string()),
            "the oldest finished run is what made room for it"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// Plant finished runs the way a pre-Stage-A-6 build did: straight into
    /// the table, with no retention pass behind them. `upsert_workflow_run`
    /// cannot express this any more, and "history that is already over the
    /// cap when the process starts" is the whole premise of the sweep.
    fn plant_finished_runs(store: &QueenStore, project: &Path, prefix: &str, count: i64) {
        let project = project_id(project).unwrap();
        let connection = store.lock();
        for n in 0..count {
            let ended = 1_000 + n;
            connection
                .execute(
                    "INSERT INTO workflow_runs(
                       run_id, project_dir, name, state, started_at_ms, ended_at_ms,
                       steps_json, error
                     ) VALUES (?1, ?2, 'demo', 'succeeded', ?3, ?4, '[]', NULL)",
                    params![format!("{prefix}-{n:05}"), project, ended - 1, ended],
                )
                .unwrap();
        }
    }

    /// v0.5.9 §2.4. Retention used to need a run to REACH a terminal state,
    /// so a project that had stopped running workflows kept every row it had
    /// ever written, forever. One sweep at startup is the only thing that
    /// reaches those, and it has to reach all of them, not just the project
    /// that happens to be loaded.
    #[test]
    fn the_startup_sweep_trims_every_project_that_has_stopped_running_workflows() {
        let (root, one, two) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        let overshoot = 4;
        plant_finished_runs(
            &store,
            &one,
            "old",
            MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT + overshoot,
        );
        plant_finished_runs(&store, &two, "other", 3);
        // A crash-interrupted run in the fat project: the sweep must not be
        // the thing that eats the resume banner.
        store
            .upsert_workflow_run(&one, "still-going", "demo", "running", 1, None, "[]")
            .unwrap();

        let swept = store.prune_every_projects_terminal_workflow_runs().unwrap();
        assert_eq!(swept, 2, "both projects with finished history are visited");

        let kept = stored_run_ids(&store, &one);
        assert_eq!(
            kept.len() as i64,
            MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT + 1,
            "trimmed to the cap, plus the running row the cap never counted"
        );
        assert!(kept.contains(&"still-going".to_string()));
        for n in 0..overshoot {
            assert!(
                !kept.contains(&format!("old-{n:05}")),
                "run {n} is older than the cap allows and should be gone"
            );
        }
        assert_eq!(
            stored_run_ids(&store, &two).len(),
            3,
            "a project under the cap loses nothing — the sweep trims, it does not clear"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// The reason the sweep reads its project list out of the table instead of
    /// putting each path back through `project_id`: a project directory that
    /// has been deleted or moved cannot be canonicalised any more, and those
    /// are precisely the installs whose rows nothing will ever come back to
    /// prune.
    #[test]
    fn the_startup_sweep_still_trims_a_project_whose_directory_is_gone() {
        let (root, one, _) = projects();
        let store = QueenStore::open_in_memory().unwrap();
        plant_finished_runs(
            &store,
            &one,
            "old",
            MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT + 2,
        );
        let ids_before = stored_run_ids(&store, &one);
        std::fs::remove_dir_all(&one).unwrap();
        assert!(
            one.canonicalize().is_err(),
            "precondition: the project path no longer resolves"
        );

        store.prune_every_projects_terminal_workflow_runs().unwrap();

        let connection = store.lock();
        let remaining: i64 = connection
            .query_row("SELECT count(*) FROM workflow_runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, MAX_TERMINAL_WORKFLOW_RUNS_PER_PROJECT);
        assert!(ids_before.len() as i64 > remaining);
        let _ = std::fs::remove_dir_all(root);
    }
}
