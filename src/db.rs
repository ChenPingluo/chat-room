use std::path::Path;

use rusqlite::Connection;

use crate::error::StartupError;

const MIGRATIONS: &[&str] = &[include_str!("../migrations/001_init.sql")];

pub fn open_db(path: &Path) -> Result<Connection, StartupError> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "
        PRAGMA journal_mode=WAL;
        PRAGMA synchronous=NORMAL;
        PRAGMA foreign_keys=OFF;
        PRAGMA busy_timeout=5000;
        PRAGMA cache_size=-2000;
        PRAGMA temp_store=MEMORY;
        PRAGMA mmap_size=0;
        PRAGMA wal_autocheckpoint=1000;
        ",
    )?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<(), StartupError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY);",
    )?;
    let current: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let version = (index as i64) + 1;
        if version <= current {
            continue;
        }
        conn.execute_batch(sql)?;
        conn.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [version],
        )?;
    }
    Ok(())
}
