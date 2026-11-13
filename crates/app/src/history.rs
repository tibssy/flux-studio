use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

#[derive(Debug, Clone)]
pub struct Generation {
    pub id: i64,
    pub prompt: String,
    pub model: String,
    pub style: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub seed: String,
    pub created_at: i64,
    pub duration_ms: i64,
    pub image_path: String,
    pub reference_path: Option<String>,
    /// `generate` or `upscale`.
    pub kind: String,
    /// Original image for an upscale entry.
    pub source_path: Option<String>,
}

pub struct NewGeneration<'a> {
    pub prompt: &'a str,
    pub model: &'a str,
    pub style: &'a str,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub seed: &'a str,
    pub created_at: i64,
    pub duration_ms: i64,
    pub image_path: &'a str,
    pub reference_path: Option<&'a str>,
    pub kind: &'a str,
    pub source_path: Option<&'a str>,
}

pub struct History {
    conn: Connection,
}

impl History {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn =
            Connection::open(path).with_context(|| format!("failed to open {}", path.display()))?;
        Self::from_connection(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS generations (
                 id          INTEGER PRIMARY KEY AUTOINCREMENT,
                 prompt      TEXT    NOT NULL,
                 model       TEXT    NOT NULL DEFAULT 'flux2-klein-4b',
                 style       TEXT    NOT NULL DEFAULT 'none',
                 width       INTEGER NOT NULL,
                 height      INTEGER NOT NULL,
                 steps       INTEGER NOT NULL,
                 seed        TEXT    NOT NULL,
                 created_at  INTEGER NOT NULL,
                 duration_ms INTEGER NOT NULL,
                 image_path  TEXT    NOT NULL,
                 reference_path TEXT,
                 kind        TEXT    NOT NULL DEFAULT 'generate',
                 source_path TEXT
             );
             CREATE INDEX IF NOT EXISTS generations_created_at
                 ON generations (created_at DESC);",
        )?;
        // Databases created before multi-model/style support lack these columns.
        ensure_column(
            &conn,
            "generations",
            "model",
            "TEXT NOT NULL DEFAULT 'flux2-klein-4b'",
        )?;
        ensure_column(
            &conn,
            "generations",
            "style",
            "TEXT NOT NULL DEFAULT 'none'",
        )?;
        ensure_column(&conn, "generations", "reference_path", "TEXT")?;
        ensure_column(
            &conn,
            "generations",
            "kind",
            "TEXT NOT NULL DEFAULT 'generate'",
        )?;
        ensure_column(&conn, "generations", "source_path", "TEXT")?;
        Ok(Self { conn })
    }

    pub fn insert(&self, generation: &NewGeneration<'_>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO generations
                 (prompt, model, style, width, height, steps, seed, created_at,
                  duration_ms, image_path, reference_path, kind, source_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                generation.prompt,
                generation.model,
                generation.style,
                generation.width,
                generation.height,
                generation.steps,
                generation.seed,
                generation.created_at,
                generation.duration_ms,
                generation.image_path,
                generation.reference_path,
                generation.kind,
                generation.source_path,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<Generation>> {
        let mut statement = self.conn.prepare(
            "SELECT id, prompt, model, style, width, height, steps, seed,
                    created_at, duration_ms, image_path, reference_path, kind,
                    source_path
             FROM generations
             ORDER BY created_at DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map([limit as i64], |row| {
            Ok(Generation {
                id: row.get(0)?,
                prompt: row.get(1)?,
                model: row.get(2)?,
                style: row.get(3)?,
                width: row.get::<_, i64>(4)? as u32,
                height: row.get::<_, i64>(5)? as u32,
                steps: row.get::<_, i64>(6)? as u32,
                seed: row.get(7)?,
                created_at: row.get(8)?,
                duration_ms: row.get(9)?,
                image_path: row.get(10)?,
                reference_path: row.get(11)?,
                kind: row.get(12)?,
                source_path: row.get(13)?,
            })
        })?;

        let mut generations = Vec::new();
        for row in rows {
            generations.push(row?);
        }
        Ok(generations)
    }

    /// Remove one entry from the database.
    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM generations WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut exists = false;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            exists = true;
            break;
        }
    }
    drop(rows);
    drop(statement);
    if !exists {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample<'a>(prompt: &'a str, model: &'a str) -> NewGeneration<'a> {
        NewGeneration {
            prompt,
            model,
            style: "none",
            width: 768,
            height: 768,
            steps: 4,
            seed: "42",
            created_at: 100,
            duration_ms: 1000,
            image_path: "/tmp/a.png",
            reference_path: None,
            kind: "generate",
            source_path: None,
        }
    }

    #[test]
    fn inserts_and_returns_newest_first() {
        let history = History::open_in_memory().unwrap();
        history.insert(&sample("first", "flux2-klein-4b")).unwrap();
        let mut second = sample("second", "flux3-klein");
        second.created_at = 101;
        history.insert(&second).unwrap();

        let recent = history.recent(10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].prompt, "second");
        assert_eq!(recent[0].model, "flux3-klein");
        assert_eq!(recent[1].prompt, "first");
    }

    #[test]
    fn respects_limit() {
        let history = History::open_in_memory().unwrap();
        for index in 0..5 {
            let mut generation = sample("x", "flux2-klein-4b");
            generation.created_at = index;
            history.insert(&generation).unwrap();
        }
        assert_eq!(history.recent(3).unwrap().len(), 3);
    }

    #[test]
    fn records_reference_path() {
        let history = History::open_in_memory().unwrap();
        let mut generation = sample("edit", "flux2-klein-4b");
        generation.reference_path = Some("/tmp/ref.png");
        history.insert(&generation).unwrap();
        let recent = history.recent(1).unwrap();
        assert_eq!(recent[0].reference_path.as_deref(), Some("/tmp/ref.png"));
    }

    #[test]
    fn records_style() {
        let history = History::open_in_memory().unwrap();
        let mut generation = sample("styled", "flux2-klein-4b");
        generation.style = "cinematic";
        history.insert(&generation).unwrap();
        assert_eq!(history.recent(1).unwrap()[0].style, "cinematic");
    }

    #[test]
    fn deletes_one_entry() {
        let history = History::open_in_memory().unwrap();
        let first = history.insert(&sample("first", "flux2-klein-4b")).unwrap();
        let mut second = sample("second", "flux2-klein-4b");
        second.created_at = 101;
        let second_id = history.insert(&second).unwrap();

        history.delete(first).unwrap();
        let recent = history.recent(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, second_id);
    }

    #[test]
    fn records_an_upscale_entry() {
        let history = History::open_in_memory().unwrap();
        let mut upscale = sample("Upscaled 4×", "mosr-4x");
        upscale.kind = "upscale";
        upscale.source_path = Some("/tmp/source.png");
        upscale.steps = 0;
        upscale.seed = "";
        history.insert(&upscale).unwrap();

        let recent = history.recent(1).unwrap();
        assert_eq!(recent[0].kind, "upscale");
        assert_eq!(recent[0].source_path.as_deref(), Some("/tmp/source.png"));
        assert_eq!(recent[0].model, "mosr-4x");
    }

    #[test]
    fn migrates_a_database_without_style_or_reference() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE generations (
                 id          INTEGER PRIMARY KEY AUTOINCREMENT,
                 prompt      TEXT    NOT NULL,
                 width       INTEGER NOT NULL,
                 height      INTEGER NOT NULL,
                 steps       INTEGER NOT NULL,
                 seed        TEXT    NOT NULL,
                 created_at  INTEGER NOT NULL,
                 duration_ms INTEGER NOT NULL,
                 image_path  TEXT    NOT NULL
             );",
        )
        .unwrap();

        let history = History::from_connection(conn).unwrap();
        history.insert(&sample("old", "flux2-klein-4b")).unwrap();
        let recent = history.recent(1).unwrap();
        assert_eq!(recent[0].style, "none");
        assert_eq!(recent[0].reference_path, None);
        assert_eq!(recent[0].kind, "generate");
        assert_eq!(recent[0].source_path, None);
    }
}
