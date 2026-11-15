use std::path::Path;

use rusqlite::Connection;

use crate::error::StartupError;

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/001_init.sql"),
    include_str!("../migrations/002_indexes.sql"),
];

pub fn open_db(path: &Path) -> Result<Connection, StartupError> {
    let mut conn = Connection::open(path)?;
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
    migrate(&mut conn)?;
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> Result<(), StartupError> {
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
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [version],
        )?;
        tx.commit()?;
    }
    Ok(())
}
