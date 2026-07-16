//! Durable, project-scoped Queen data (Phase 3.6–3.7).

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, types::Type, Connection, OptionalExtension, Row, TransactionBehavior};
use serde::Serialize;

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

pub struct QueenStore {
    connection: Mutex<Connection>,
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
        Self::from_connection(connection, true)
    }

    #[cfg(test)]
    fn open_in_memory() -> Result<Self, String> {
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
        if version > 2 {
            return Err(format!(
                "unsupported Queen database version {version} (expected 2)"
            ));
        }
        if version == 0 {
            let schema_result = connection.execute_batch(
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
                 PRAGMA user_version = 2;
                 COMMIT;",
            );
            if let Err(error) = schema_result {
                let _ = connection.execute_batch("ROLLBACK;");
                return Err(format!("cannot initialize Queen database: {error}"));
            }
        } else if version == 1 {
            let migration_result = connection.execute_batch(
                "BEGIN IMMEDIATE;
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
                 CREATE INDEX inbox_recipient_id
                   ON inbox_messages(project_dir, recipient, id ASC);
                 CREATE INDEX inbox_root_id
                   ON inbox_messages(project_dir, root_message_id, id ASC);
                 PRAGMA user_version = 2;
                 COMMIT;",
            );
            if let Err(error) = migration_result {
                let _ = connection.execute_batch("ROLLBACK;");
                return Err(format!(
                    "cannot migrate Queen database to version 2: {error}"
                ));
            }
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        match self.connection.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
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
        Ok(reply)
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
        let message = store
            .send_inbox(
                &one,
                "claude-impl".to_string(),
                "codex-review".to_string(),
                "Review session storage".to_string(),
                "Please inspect the migration.".to_string(),
            )
            .unwrap();
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
        assert_eq!(version, 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_unknown_database_versions() {
        let (root, _, _) = projects();
        let database = root.join("data/queen.sqlite3");
        std::fs::create_dir_all(database.parent().unwrap()).unwrap();
        let connection = Connection::open(&database).unwrap();
        connection.pragma_update(None, "user_version", 3).unwrap();
        drop(connection);
        let error = QueenStore::open(&database).err().unwrap();
        assert!(error.contains("unsupported Queen database version"));
        let _ = std::fs::remove_dir_all(root);
    }
}
