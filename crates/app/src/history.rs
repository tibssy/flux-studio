use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

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
    /// All reference images used for the run, in prompt order.
    pub reference_paths: Vec<String>,
    /// `generate`, `upscale`, `colorize`, or `expand`.
    pub kind: String,
    /// Original image for an upscale entry.
    pub source_path: Option<String>,
    /// Entry whose result was used as the input image, when there is one.
    pub parent_id: Option<i64>,
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
    /// All reference images used for the run, in prompt order.
    pub reference_paths: &'a [String],
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
                 reference_paths TEXT,
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
        ensure_column(&conn, "generations", "reference_paths", "TEXT")?;
        ensure_column(
            &conn,
            "generations",
            "kind",
            "TEXT NOT NULL DEFAULT 'generate'",
        )?;
        ensure_column(&conn, "generations", "source_path", "TEXT")?;
        let added_parent = ensure_column(&conn, "generations", "parent_id", "INTEGER")?;
        if added_parent {
            // Older rows only kept the input path; resolve it to the entry that
            // produced that image so existing history groups retroactively.
            conn.execute(
                "UPDATE generations
                 SET parent_id = (
                     SELECT p.id FROM generations p
                     WHERE p.image_path = COALESCE(generations.reference_path,
                                                  generations.source_path)
                       AND p.id != generations.id
                     ORDER BY p.id DESC
                     LIMIT 1
                 )
                 WHERE COALESCE(reference_path, source_path) IS NOT NULL",
                [],
            )?;
        }
        Ok(Self { conn })
    }

    pub fn insert(&self, generation: &NewGeneration<'_>) -> Result<(i64, Option<i64>)> {
        let parent_id = match generation.reference_path.or(generation.source_path) {
            Some(path) => self
                .conn
                .query_row(
                    "SELECT id FROM generations WHERE image_path = ?1
                     ORDER BY id DESC LIMIT 1",
                    [path],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?,
            None => None,
        };
        let reference_paths = if generation.reference_paths.is_empty() {
            None
        } else {
            Some(serde_json::to_string(generation.reference_paths)?)
        };
        self.conn.execute(
            "INSERT INTO generations
                 (prompt, model, style, width, height, steps, seed, created_at,
                  duration_ms, image_path, reference_path, reference_paths, kind,
                  source_path, parent_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
                reference_paths,
                generation.kind,
                generation.source_path,
                parent_id,
            ],
        )?;
        Ok((self.conn.last_insert_rowid(), parent_id))
    }

    pub fn recent(&self, limit: usize) -> Result<Vec<Generation>> {
        let mut statement = self.conn.prepare(
            "SELECT id, prompt, model, style, width, height, steps, seed,
                    created_at, duration_ms, image_path, reference_path,
                    reference_paths, kind, source_path, parent_id
             FROM generations
             ORDER BY created_at DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map([limit as i64], |row| {
            let reference_path: Option<String> = row.get(11)?;
            let raw_paths: Option<String> = row.get(12)?;
            let mut reference_paths = raw_paths
                .as_deref()
                .and_then(|paths| serde_json::from_str::<Vec<String>>(paths).ok())
                .unwrap_or_default();
            if reference_paths.is_empty() {
                // Databases written before multi-reference support only kept
                // the first path.
                if let Some(path) = reference_path.clone() {
                    reference_paths.push(path);
                }
            }
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
                reference_path,
                reference_paths,
                kind: row.get(13)?,
                source_path: row.get(14)?,
                parent_id: row.get(15)?,
            })
        })?;

        let mut generations = Vec::new();
        for row in rows {
            generations.push(row?);
        }
        Ok(generations)
    }

    /// Remove one entry from the database.
    ///
    /// Children are re-parented to the deleted entry's parent so a chain keeps
    /// its shape when a middle step is removed.
    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE generations
             SET parent_id = (SELECT parent_id FROM generations WHERE id = ?1)
             WHERE parent_id = ?1",
            [id],
        )?;
        self.conn
            .execute("DELETE FROM generations WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<bool> {
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
    Ok(!exists)
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
            reference_paths: &[],
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
        assert_eq!(recent[0].reference_paths, vec!["/tmp/ref.png"]);
    }

    #[test]
    fn records_multiple_reference_paths() {
        let history = History::open_in_memory().unwrap();
        let mut generation = sample("edit", "flux2-klein-4b");
        generation.reference_path = Some("/tmp/a.png");
        let paths = ["/tmp/a.png".to_string(), "/tmp/b.png".to_string()];
        generation.reference_paths = &paths;
        history.insert(&generation).unwrap();
        let recent = history.recent(1).unwrap();
        assert_eq!(recent[0].reference_path.as_deref(), Some("/tmp/a.png"));
        assert_eq!(
            recent[0].reference_paths,
            vec!["/tmp/a.png".to_string(), "/tmp/b.png".to_string()]
        );
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
        let first = history
            .insert(&sample("first", "flux2-klein-4b"))
            .unwrap()
            .0;
        let mut second = sample("second", "flux2-klein-4b");
        second.created_at = 101;
        let second_id = history.insert(&second).unwrap().0;

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
    fn resolves_parent_for_reference_and_source() {
        let history = History::open_in_memory().unwrap();
        let root = sample("root", "flux2-klein-4b");
        let root_id = history.insert(&root).unwrap().0;

        let mut edit = sample("edit", "flux2-klein-4b");
        edit.created_at = 101;
        edit.image_path = "/tmp/edit.png";
        edit.reference_path = Some("/tmp/a.png");
        let (edit_id, parent) = history.insert(&edit).unwrap();
        assert_eq!(parent, Some(root_id));

        let mut upscale = sample("upscale", "mosr-4x");
        upscale.created_at = 102;
        upscale.kind = "upscale";
        upscale.image_path = "/tmp/upscaled.png";
        upscale.source_path = Some("/tmp/edit.png");
        let (_, upscale_parent) = history.insert(&upscale).unwrap();
        assert_eq!(upscale_parent, Some(edit_id));

        // A reference that is not in the history starts a new chain.
        let mut external = sample("external", "flux2-klein-4b");
        external.created_at = 103;
        external.image_path = "/tmp/external.png";
        external.reference_path = Some("/tmp/elsewhere.png");
        assert_eq!(history.insert(&external).unwrap().1, None);
    }

    #[test]
    fn delete_reparents_children() {
        let history = History::open_in_memory().unwrap();
        let root_id = history.insert(&sample("root", "flux2-klein-4b")).unwrap().0;

        let mut middle = sample("middle", "flux2-klein-4b");
        middle.created_at = 101;
        middle.image_path = "/tmp/middle.png";
        middle.reference_path = Some("/tmp/a.png");
        let middle_id = history.insert(&middle).unwrap().0;

        let mut leaf = sample("leaf", "flux2-klein-4b");
        leaf.created_at = 102;
        leaf.image_path = "/tmp/leaf.png";
        leaf.reference_path = Some("/tmp/middle.png");
        let leaf_id = history.insert(&leaf).unwrap().0;

        history.delete(middle_id).unwrap();
        let recent = history.recent(10).unwrap();
        let leaf = recent.iter().find(|entry| entry.id == leaf_id).unwrap();
        assert_eq!(leaf.parent_id, Some(root_id));
    }

    #[test]
    fn backfills_parent_ids_for_existing_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE generations (
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
             INSERT INTO generations
                 (prompt, width, height, steps, seed, created_at, duration_ms,
                  image_path)
             VALUES ('root', 768, 768, 4, '1', 100, 1000, '/tmp/a.png');
             INSERT INTO generations
                 (prompt, width, height, steps, seed, created_at, duration_ms,
                  image_path, reference_path)
             VALUES ('edit', 768, 768, 4, '2', 101, 1000, '/tmp/b.png',
                     '/tmp/a.png');",
        )
        .unwrap();

        let history = History::from_connection(conn).unwrap();
        let recent = history.recent(10).unwrap();
        let edit = recent.iter().find(|entry| entry.prompt == "edit").unwrap();
        let root = recent.iter().find(|entry| entry.prompt == "root").unwrap();
        assert_eq!(edit.parent_id, Some(root.id));
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
