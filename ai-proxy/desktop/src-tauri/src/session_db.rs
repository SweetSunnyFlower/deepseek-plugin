use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub cli_session_id: String,
    pub provider: String,
    pub web_chat_id: String,
    pub web_url: String,
    pub synced_count: usize,
    pub retryable: bool,
}

impl Segment {
    pub fn new(
        cli_session_id: &str,
        provider: &str,
        web_chat_id: &str,
        web_url: &str,
        synced_count: usize,
    ) -> Self {
        Self {
            cli_session_id: cli_session_id.to_string(),
            provider: provider.to_string(),
            web_chat_id: web_chat_id.to_string(),
            web_url: web_url.to_string(),
            synced_count,
            retryable: false,
        }
    }
}

pub struct SessionDb {
    conn: Mutex<Connection>,
}

impl SessionDb {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS session_segments (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                cli_session_id TEXT NOT NULL,
                provider       TEXT NOT NULL,
                web_chat_id    TEXT NOT NULL,
                web_url        TEXT NOT NULL,
                synced_count   INTEGER NOT NULL,
                retryable      INTEGER NOT NULL DEFAULT 0,
                created_at     TEXT NOT NULL,
                updated_at     TEXT NOT NULL,
                UNIQUE (cli_session_id, provider, web_chat_id)
            );
            CREATE INDEX IF NOT EXISTS idx_segments_session
                ON session_segments (cli_session_id, provider, updated_at DESC);",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn upsert_segment(&self, seg: &Segment) -> Result<(), String> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO session_segments
                (cli_session_id, provider, web_chat_id, web_url, synced_count, retryable, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(cli_session_id, provider, web_chat_id)
             DO UPDATE SET
                web_url = excluded.web_url,
                synced_count = excluded.synced_count,
                retryable = excluded.retryable,
                updated_at = excluded.updated_at",
            params![
                seg.cli_session_id,
                seg.provider,
                seg.web_chat_id,
                seg.web_url,
                seg.synced_count as i64,
                seg.retryable as i64,
                now,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn latest_segment(&self, cli_session_id: &str, provider: &str) -> Result<Option<Segment>, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT cli_session_id, provider, web_chat_id, web_url, synced_count, retryable
             FROM session_segments
             WHERE cli_session_id = ?1 AND provider = ?2
             ORDER BY updated_at DESC, id DESC
             LIMIT 1",
            params![cli_session_id, provider],
            |row| {
                Ok(Segment {
                    cli_session_id: row.get(0)?,
                    provider: row.get(1)?,
                    web_chat_id: row.get(2)?,
                    web_url: row.get(3)?,
                    synced_count: row.get::<_, i64>(4)? as usize,
                    retryable: row.get::<_, i64>(5)? != 0,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_and_reads_latest_segment() {
        let db = SessionDb::open_in_memory().unwrap();
        db.upsert_segment(&Segment::new(
            "cli-1",
            "deepseek",
            "chat-a",
            "https://chat.deepseek.com/a/chat/s/chat-a",
            2,
        ))
        .unwrap();
        db.upsert_segment(&Segment::new(
            "cli-1",
            "deepseek",
            "chat-b",
            "https://chat.deepseek.com/a/chat/s/chat-b",
            5,
        ))
        .unwrap();
        let latest = db.latest_segment("cli-1", "deepseek").unwrap().unwrap();
        assert_eq!(latest.web_chat_id, "chat-b");
        assert_eq!(latest.synced_count, 5);
    }

    #[test]
    fn same_chat_id_updates_instead_of_duplicating() {
        let db = SessionDb::open_in_memory().unwrap();
        let url = "https://chat.deepseek.com/a/chat/s/chat-a";
        db.upsert_segment(&Segment::new("cli-1", "deepseek", "chat-a", url, 2))
            .unwrap();
        let mut seg = Segment::new("cli-1", "deepseek", "chat-a", url, 4);
        seg.retryable = true;
        db.upsert_segment(&seg).unwrap();
        let rows: i64 = {
            let conn = db.conn.lock().unwrap();
            conn.query_row("SELECT COUNT(*) FROM session_segments", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(rows, 1);
        let latest = db.latest_segment("cli-1", "deepseek").unwrap().unwrap();
        assert_eq!(latest.synced_count, 4);
        assert!(latest.retryable);
    }

    #[test]
    fn provider_is_isolated() {
        let db = SessionDb::open_in_memory().unwrap();
        db.upsert_segment(&Segment::new(
            "cli-1",
            "deepseek",
            "chat-a",
            "https://chat.deepseek.com/a/chat/s/chat-a",
            2,
        ))
        .unwrap();
        assert!(db.latest_segment("cli-1", "chatgpt").unwrap().is_none());
        assert!(db.latest_segment("cli-2", "deepseek").unwrap().is_none());
    }
}
