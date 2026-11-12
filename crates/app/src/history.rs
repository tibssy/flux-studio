use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

#[derive(Debug, Clone)]
pub struct Generation {
    pub id: i64,
    pub prompt: String,
    pub model: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub seed: String,
    pub created_at: i64,
    pub duration_ms: i64,
    pub image_path: String,
    pub reference_path: Option<String>,
}

pub struct NewGeneration<'a> {
    pub prompt: &'a str,
    pub model: &'a str,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub seed: &'a str,
    pub created_at: i64,
    pub duration_ms: i64,
    pub image_path: &'a str,
    pub reference_path: Option<&'a str>,
}

pub struct History {
    conn: Connection,
}

impl History {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open {}", path.display()))?;
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
                 width       INTEGER NOT NULL,
                 height      INTEGER NOT NULL,
                 steps       INTEGER NOT NULL,
                 seed        TEXT    NOT NULL,
                 created_at  INTEGER NOT NULL,
                 duration_ms INTEGER NOT NULL,
                 image_path  TEXT    NOT NULL,
                 reference_path TEXT
             );
             CREATE INDEX IF NOT EXISTS generations_created_at
                 ON generations (created_at DESC);",
        )?;
        // Databases created before multi-model support lack these columns.
        ensure_column(&conn, "generations", "model", "TEXT NOT NULL DEFAULT 'flux2-klein-4b'")?;
        ensure_column(&conn, "generations", "reference_path", "TEXT")?;
        Ok(Self { conn })
    }

    pub fn insert(&self, generation: &NewGeneration<'_>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO generations
                 (prompt, model, width, height, steps, seed, created_at, duration_ms,
                  image_path, reference_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                generation.prompt,
                generation.model,
                generation.width,
                generation.height,
                generation.steps,
                generation.seed,
                generation.created_at,
                generation.duration_ms,
                generation.image_path,
                generation.reference_path,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<Generation>> {
        let mut statement = self.conn.prepare(
            "SELECT id, prompt, model, width, height, steps, seed, created_at,
                    duration_ms, image_path, reference_path
             FROM generations
             ORDER BY created_at DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map([limit as i64], |row| {
            Ok(Generation {
                id: row.get(0)?,
                prompt: row.get(1)?,
                model: row.get(2)?,
                width: row.get::<_, i64>(3)? as u32,
                height: row.get::<_, i64>(4)? as u32,
                steps: row.get::<_, i64>(5)? as u32,
                seed: row.get(6)?,
                created_at: row.get(7)?,
                duration_ms: row.get(8)?,
                image_path: row.get(9)?,
                reference_path: row.get(10)?,
            })
        })?;

        let mut generations = Vec::new();
        for row in rows {
            generations.push(row?);
        }
        Ok(generations)
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
            width: 768,
            height: 768,
            steps: 4,
            seed: "42",
            created_at: 100,
            duration_ms: 1000,
            image_path: "/tmp/a.png",
            reference_path: None,
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
}
