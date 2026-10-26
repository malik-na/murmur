//! SQLite persistence for history and dictionary, plus settings.json.

use crate::types::*;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Mutex;

const SCHEMA_VERSION: i32 = 2;
const DICT_COLS: &str =
    "id, phrase, replacement, is_snippet, is_starred, frequency_used, created_at, modified_at, source, heard_as";
/// Statuses that count as a real dictation for stats.
const COUNTED: &str = "status IN ('formatted', 'raw_transcript')";

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Db> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Db> {
        let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS history (
                    id TEXT PRIMARY KEY, timestamp TEXT NOT NULL, tz_offset_minutes INTEGER NOT NULL,
                    app TEXT, asr_text TEXT NOT NULL, formatted_text TEXT NOT NULL, duration REAL NOT NULL,
                    num_words INTEGER NOT NULL, status TEXT NOT NULL, language TEXT);
                 CREATE INDEX IF NOT EXISTS history_timestamp ON history(timestamp);
                 CREATE TABLE IF NOT EXISTS dictionary (
                    id TEXT PRIMARY KEY, phrase TEXT NOT NULL, replacement TEXT,
                    is_snippet INTEGER NOT NULL DEFAULT 0, is_starred INTEGER NOT NULL DEFAULT 0,
                    frequency_used INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL,
                    modified_at TEXT NOT NULL, is_deleted INTEGER NOT NULL DEFAULT 0);
                 CREATE UNIQUE INDEX IF NOT EXISTS dictionary_phrase ON dictionary(lower(phrase), is_snippet);",
            )?;
        }
        migrate(&conn)?;
        Ok(Db { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn insert_history(&self, e: &HistoryEntry) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO history (id, timestamp, tz_offset_minutes, app, asr_text, formatted_text,
             duration, num_words, status, language) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                e.id, e.timestamp, e.tz_offset_minutes, e.app, e.asr_text, e.formatted_text,
                e.duration as f64, e.num_words, e.status, e.language
            ],
        )?;
        Ok(())
    }

    /// Newest first. `search` matches asr_text, formatted_text or app, case-insensitively.
    pub fn recent_history(&self, limit: usize, search: Option<&str>) -> Result<Vec<HistoryEntry>> {
        let pat = search.map(str::trim).filter(|s| !s.is_empty()).map(|q| {
            format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"))
        });
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT id, timestamp, tz_offset_minutes, app, asr_text, formatted_text, duration, num_words, status,
             language FROM history
             WHERE ?1 IS NULL OR asr_text LIKE ?1 ESCAPE '\\' OR formatted_text LIKE ?1 ESCAPE '\\'
                OR app LIKE ?1 ESCAPE '\\'
             ORDER BY timestamp DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![pat, limit.min(i64::MAX as usize) as i64], history_row)?;
        let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn last_history(&self) -> Result<Option<HistoryEntry>> {
        Ok(self.recent_history(1, None)?.into_iter().next())
    }

    pub fn delete_history(&self, id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM history WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn clear_history(&self) -> Result<()> {
        self.conn().execute("DELETE FROM history", [])?;
        Ok(())
    }

    pub fn stats(&self) -> Result<Stats> {
        let conn = self.conn();
        let (total_words, total_dictations): (i64, i64) = conn.query_row(
            &format!("SELECT COALESCE(SUM(num_words), 0), COUNT(*) FROM history WHERE {COUNTED}"),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let (words, secs): (f64, f64) = conn.query_row(
            "SELECT COALESCE(SUM(num_words), 0), COALESCE(SUM(duration), 0) FROM (
                SELECT num_words, duration FROM history WHERE duration < 400 AND num_words > 0
                ORDER BY timestamp DESC LIMIT 100)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let average_wpm = if secs > 0.0 { (words / (secs / 60.0)) as f32 } else { 0.0 };

        let mut st = conn.prepare(&format!("SELECT timestamp, tz_offset_minutes FROM history WHERE {COUNTED}"))?;
        let days: BTreeSet<NaiveDate> = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i32>(1)?)))?
            .filter_map(|r| r.ok())
            .filter_map(|(ts, off)| local_day(&ts, off))
            .collect();
        let today = chrono::Local::now().date_naive();
        Ok(Stats {
            total_words: total_words.max(0) as u64,
            total_dictations: total_dictations.max(0) as u64,
            average_wpm,
            day_streak: streak(&days, today),
        })
    }

    /// Excludes soft-deleted entries; starred first, then by phrase.
    pub fn list_dictionary(&self, snippets: bool) -> Result<Vec<DictEntry>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            &format!(
                "SELECT {DICT_COLS} FROM dictionary WHERE is_deleted = 0 AND is_snippet = ?1
                 ORDER BY is_starred DESC, lower(phrase)"
            ),
        )?;
        let rows = st.query_map([snippets], dict_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Unique on lower(phrase)+is_snippet; an existing (even soft-deleted) entry is updated and restored.
    pub fn upsert_dict(&self, phrase: &str, replacement: Option<&str>, is_snippet: bool) -> Result<DictEntry> {
        let phrase = phrase.trim();
        anyhow::ensure!(!phrase.is_empty(), "phrase is empty");
        let replacement = replacement.map(str::trim).filter(|r| !r.is_empty());
        let now = Utc::now().to_rfc3339();
        let conn = self.conn();
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM dictionary WHERE lower(phrase) = lower(?1) AND is_snippet = ?2",
                params![phrase, is_snippet],
                |r| r.get(0),
            )
            .optional()?;
        let id = match existing {
            Some(id) => {
                conn.execute(
                    "UPDATE dictionary SET phrase = ?2, replacement = ?3, is_deleted = 0, modified_at = ?4 WHERE id = ?1",
                    params![id, phrase, replacement, now],
                )?;
                id
            }
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO dictionary (id, phrase, replacement, is_snippet, is_starred, frequency_used,
                     created_at, modified_at, is_deleted) VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, ?5, 0)",
                    params![id, phrase, replacement, is_snippet, now],
                )?;
                id
            }
        };
        Ok(conn.query_row(
            &format!("SELECT {DICT_COLS} FROM dictionary WHERE id = ?1"),
            [&id],
            dict_row,
        )?)
    }

    pub fn delete_dict(&self, id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE dictionary SET is_deleted = 1, modified_at = ?2 WHERE id = ?1",
            params![id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn set_starred(&self, id: &str, starred: bool) -> Result<()> {
        self.conn().execute(
            "UPDATE dictionary SET is_starred = ?2, modified_at = ?3 WHERE id = ?1",
            params![id, starred, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Adds a word learned from a user edit. Returns None when the phrase already exists
    /// (live or soft-deleted: a deliberate removal is never resurrected).
    pub fn add_learned(&self, word: &str, heard_as: &str) -> Result<Option<DictEntry>> {
        let word = word.trim();
        anyhow::ensure!(!word.is_empty(), "word is empty");
        let heard_as = heard_as.trim();
        let conn = self.conn();
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM dictionary WHERE lower(phrase) = lower(?1) AND is_snippet = 0)",
            [word],
            |r| r.get(0),
        )?;
        if exists {
            return Ok(None);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let heard = Some(heard_as).filter(|h| !h.is_empty());
        conn.execute(
            "INSERT INTO dictionary (id, phrase, replacement, is_snippet, is_starred, frequency_used,
             created_at, modified_at, is_deleted, source, heard_as)
             VALUES (?1, ?2, NULL, 0, 0, 0, ?3, ?3, 0, 'user_edits', ?4)",
            params![id, word, now, heard],
        )?;
        Ok(Some(conn.query_row(
            &format!("SELECT {DICT_COLS} FROM dictionary WHERE id = ?1"),
            [&id],
            dict_row,
        )?))
    }

    pub fn bump_usage(&self, ids: &[String]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for id in ids {
            tx.execute("UPDATE dictionary SET frequency_used = frequency_used + 1 WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn history_row(r: &Row) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: r.get(0)?,
        timestamp: r.get(1)?,
        tz_offset_minutes: r.get(2)?,
        app: r.get(3)?,
        asr_text: r.get(4)?,
        formatted_text: r.get(5)?,
        duration: r.get::<_, f64>(6)? as f32,
        num_words: r.get(7)?,
        status: r.get(8)?,
        language: r.get(9)?,
    })
}

fn dict_row(r: &Row) -> rusqlite::Result<DictEntry> {
    Ok(DictEntry {
        id: r.get(0)?,
        phrase: r.get(1)?,
        replacement: r.get(2)?,
        is_snippet: r.get(3)?,
        is_starred: r.get(4)?,
        frequency_used: r.get(5)?,
        created_at: r.get(6)?,
        modified_at: r.get(7)?,
        source: r.get(8)?,
        heard_as: r.get(9)?,
    })
}

/// Idempotent upgrade to the current schema version; safe on fresh and existing DBs.
fn migrate(conn: &Connection) -> Result<()> {
    let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version >= SCHEMA_VERSION {
        return Ok(());
    }
    let has_col = |name: &str| -> Result<bool> {
        let mut st = conn.prepare("SELECT name FROM pragma_table_info('dictionary')")?;
        let names = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(names.iter().any(|n| n == name))
    };
    if !has_col("source")? {
        conn.execute("ALTER TABLE dictionary ADD COLUMN source TEXT NOT NULL DEFAULT 'manual'", [])?;
    }
    if !has_col("heard_as")? {
        conn.execute("ALTER TABLE dictionary ADD COLUMN heard_as TEXT", [])?;
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

/// Calendar day of a UTC RFC 3339 timestamp in the timezone it was recorded in.
fn local_day(ts: &str, offset_minutes: i32) -> Option<NaiveDate> {
    let utc = DateTime::parse_from_rfc3339(ts).ok()?.with_timezone(&Utc);
    let tz = FixedOffset::east_opt(offset_minutes.checked_mul(60)?)?;
    Some(utc.with_timezone(&tz).date_naive())
}

/// Consecutive days with a dictation, ending today or yesterday.
fn streak(days: &BTreeSet<NaiveDate>, today: NaiveDate) -> u32 {
    let mut day = if days.contains(&today) { today } else { today - Duration::days(1) };
    let mut n = 0;
    while days.contains(&day) {
        n += 1;
        day -= Duration::days(1);
    }
    n
}

fn settings_path() -> std::path::PathBuf {
    data_dir().join("settings.json")
}

pub fn load_settings() -> Settings {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(s: &Settings) -> Result<()> {
    let path = settings_path();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, ts: DateTime<Utc>, offset: i32, words: u32, duration: f32, status: &str) -> HistoryEntry {
        HistoryEntry {
            id: id.into(),
            timestamp: ts.to_rfc3339(),
            tz_offset_minutes: offset,
            app: Some("Notes".into()),
            asr_text: format!("raw {id}"),
            formatted_text: format!("Formatted {id}"),
            duration,
            num_words: words,
            status: status.into(),
            language: Some("en".into()),
        }
    }

    #[test]
    fn upsert_is_unique_case_insensitively() {
        let db = Db::open_in_memory().unwrap();
        let a = db.upsert_dict("Kubernetes", None, false).unwrap();
        let b = db.upsert_dict("kubernetes", Some("K8s"), false).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(b.replacement.as_deref(), Some("K8s"));
        // Same phrase as a snippet is a separate entry.
        let s = db.upsert_dict("kubernetes", Some("expanded"), true).unwrap();
        assert_ne!(s.id, a.id);
        assert_eq!(db.list_dictionary(false).unwrap().len(), 1);
        assert_eq!(db.list_dictionary(true).unwrap().len(), 1);
    }

    #[test]
    fn soft_delete_hides_and_upsert_restores() {
        let db = Db::open_in_memory().unwrap();
        let a = db.upsert_dict("Murmur", None, false).unwrap();
        db.bump_usage(&[a.id.clone()]).unwrap();
        db.delete_dict(&a.id).unwrap();
        assert!(db.list_dictionary(false).unwrap().is_empty());
        let b = db.upsert_dict("murmur", Some("Murmur"), false).unwrap();
        assert_eq!(b.id, a.id);
        assert_eq!(b.frequency_used, 1);
        assert_eq!(db.list_dictionary(false).unwrap(), vec![b]);
    }

    #[test]
    fn starred_sort_first() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_dict("alpha", None, false).unwrap();
        let z = db.upsert_dict("zulu", None, false).unwrap();
        db.set_starred(&z.id, true).unwrap();
        let names: Vec<_> = db.list_dictionary(false).unwrap().into_iter().map(|e| e.phrase).collect();
        assert_eq!(names, ["zulu", "alpha"]);
    }

    #[test]
    fn history_search_and_order() {
        let db = Db::open_in_memory().unwrap();
        let now = Utc::now();
        db.insert_history(&entry("a", now - Duration::minutes(5), 0, 3, 2.0, "formatted")).unwrap();
        db.insert_history(&entry("b", now, 0, 3, 2.0, "formatted")).unwrap();
        let all = db.recent_history(10, None).unwrap();
        assert_eq!(all.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(db.recent_history(10, Some("FORMATTED A")).unwrap().len(), 1);
        assert_eq!(db.recent_history(10, Some("notes")).unwrap().len(), 2);
        assert_eq!(db.recent_history(10, Some("50%")).unwrap().len(), 0);
        assert_eq!(db.last_history().unwrap().unwrap().id, "b");
        db.delete_history("b").unwrap();
        assert_eq!(db.last_history().unwrap().unwrap().id, "a");
        db.clear_history().unwrap();
        assert!(db.last_history().unwrap().is_none());
    }

    #[test]
    fn stats_wpm_excludes_long_and_wordless() {
        let db = Db::open_in_memory().unwrap();
        let now = Utc::now();
        db.insert_history(&entry("a", now, 0, 60, 30.0, "formatted")).unwrap(); // 120 wpm
        db.insert_history(&entry("b", now, 0, 40, 30.0, "formatted")).unwrap(); // together: 100 words / 1 min
        db.insert_history(&entry("long", now, 0, 1000, 500.0, "formatted")).unwrap();
        db.insert_history(&entry("empty", now, 0, 0, 5.0, "empty")).unwrap();
        let s = db.stats().unwrap();
        assert!((s.average_wpm - 100.0).abs() < 0.01, "{}", s.average_wpm);
        assert_eq!(s.total_words, 1100);
        assert_eq!(s.total_dictations, 3);
        assert_eq!(s.day_streak, 1);
        assert_eq!(Db::open_in_memory().unwrap().stats().unwrap().average_wpm, 0.0);
    }

    #[test]
    fn streak_counts_consecutive_days_and_stops_at_gap() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let today = d("2026-10-26");
        let days: BTreeSet<_> = ["2026-10-26", "2026-10-25", "2026-10-24", "2026-10-21"].map(d).into();
        assert_eq!(streak(&days, today), 3);
        // Ending yesterday still counts; ending two days ago doesn't.
        assert_eq!(streak(&days, d("2026-10-27")), 3);
        assert_eq!(streak(&days, d("2026-10-28")), 0);
        assert_eq!(streak(&BTreeSet::new(), today), 0);
    }

    #[test]
    fn local_day_uses_recorded_offset() {
        // 02:00 UTC is still the previous evening in New York (-240 min).
        assert_eq!(local_day("2026-10-26T02:00:00+00:00", -240), NaiveDate::from_ymd_opt(2026, 10, 25));
        assert_eq!(local_day("2026-10-26T02:00:00+00:00", 60), NaiveDate::from_ymd_opt(2026, 10, 26));
    }

    #[test]
    fn stats_streak_from_rows() {
        let db = Db::open_in_memory().unwrap();
        let off = chrono::Local::now().offset().local_minus_utc() / 60;
        let now = Utc::now();
        for (i, days_ago) in [0, 1, 3].iter().enumerate() {
            let e = entry(&i.to_string(), now - Duration::days(*days_ago), off, 5, 3.0, "formatted");
            db.insert_history(&e).unwrap();
        }
        assert_eq!(db.stats().unwrap().day_streak, 2);
    }

    #[test]
    fn migrates_v1_db_preserving_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE dictionary (
                id TEXT PRIMARY KEY, phrase TEXT NOT NULL, replacement TEXT,
                is_snippet INTEGER NOT NULL DEFAULT 0, is_starred INTEGER NOT NULL DEFAULT 0,
                frequency_used INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL,
                modified_at TEXT NOT NULL, is_deleted INTEGER NOT NULL DEFAULT 0);
             CREATE UNIQUE INDEX dictionary_phrase ON dictionary(lower(phrase), is_snippet);
             INSERT INTO dictionary VALUES ('x', 'Kubernetes', NULL, 0, 1, 4, 't', 't', 0);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        let db = Db::init(conn).unwrap();
        let v: i32 = db.conn().pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, 2);
        let list = db.list_dictionary(false).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].phrase, "Kubernetes");
        assert_eq!(list[0].source, "manual");
        assert_eq!(list[0].heard_as, None);
        assert!(list[0].is_starred);
        assert_eq!(list[0].frequency_used, 4);
        // Running again is a no-op.
        migrate(&db.conn()).unwrap();
    }

    #[test]
    fn fresh_db_is_v2() {
        let db = Db::open_in_memory().unwrap();
        let v: i32 = db.conn().pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, 2);
        assert_eq!(db.upsert_dict("a", None, false).unwrap().source, "manual");
    }

    #[test]
    fn add_learned_inserts_then_skips_known_and_deleted() {
        let db = Db::open_in_memory().unwrap();
        let e = db.add_learned("Claude Code", "Cloud Code").unwrap().unwrap();
        assert_eq!(e.phrase, "Claude Code");
        assert_eq!(e.source, "user_edits");
        assert_eq!(e.heard_as.as_deref(), Some("Cloud Code"));
        assert_eq!(e.replacement, None);
        assert!(!e.is_starred);
        assert_eq!(db.list_dictionary(false).unwrap(), vec![e.clone()]);
        // Already present (case-insensitive): untouched.
        assert!(db.add_learned("claude code", "clod code").unwrap().is_none());
        assert_eq!(db.list_dictionary(false).unwrap(), vec![e.clone()]);
        // Soft-deleted: not resurrected.
        db.delete_dict(&e.id).unwrap();
        assert!(db.add_learned("Claude Code", "Cloud Code").unwrap().is_none());
        assert!(db.list_dictionary(false).unwrap().is_empty());
        // A snippet with the same phrase doesn't block a learned word.
        db.upsert_dict("Priya", Some("expansion"), true).unwrap();
        assert!(db.add_learned("Priya", "Pria").unwrap().is_some());
    }
}
