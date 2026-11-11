//! SQLite incluso nel binario, in modalità WAL: le letture dell'interfaccia non bloccano
//! le scritture della sync (sez. 3).
//!
//! Una connessione per scrivere e alcune per leggere. Le chiamate da codice async passano
//! da `spawn_blocking`, così il runtime non si ferma mai su SQLite.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::Result;

const MIGRATIONS: &[&str] = &[include_str!("schema_001.sql"), include_str!("schema_002.sql"), include_str!("schema_003.sql"), include_str!("schema_004.sql"), include_str!("schema_005.sql")];
const READERS: usize = 3;

pub struct Db {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    next_reader: AtomicUsize,
}

impl Db {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| crate::Error::other(format!("cartella dati: {e}")))?;
        }
        let mut writer = Connection::open(path)?;
        configure(&writer)?;
        migrate(&mut writer)?;
        let readers = (0..READERS)
            .map(|_| {
                let c = Connection::open(path)?;
                configure(&c)?;
                c.pragma_update(None, "query_only", true)?;
                Ok(Mutex::new(c))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Arc::new(Db { path: path.to_owned(), writer: Mutex::new(writer), readers, next_reader: AtomicUsize::new(0) }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Scrittura sincrona. Da codice async usare [`Db::write_async`].
    pub fn write<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut conn = self.writer.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut conn)
    }

    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let i = self.next_reader.fetch_add(1, Ordering::Relaxed) % self.readers.len();
        let conn = self.readers[i].lock().unwrap_or_else(|p| p.into_inner());
        f(&conn)
    }

    pub async fn write_async<T, F>(self: &Arc<Self>, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let db = Arc::clone(self);
        tokio::task::spawn_blocking(move || db.write(f)).await?
    }

    pub async fn read_async<T, F>(self: &Arc<Self>, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let db = Arc::clone(self);
        tokio::task::spawn_blocking(move || db.read(f)).await?
    }
}

fn configure(c: &Connection) -> Result<()> {
    c.pragma_update(None, "journal_mode", "WAL")?;
    c.pragma_update(None, "synchronous", "NORMAL")?;
    c.pragma_update(None, "foreign_keys", true)?;
    c.pragma_update(None, "busy_timeout", 5000)?;
    c.pragma_update(None, "temp_store", "MEMORY")?;
    c.pragma_update(None, "cache_size", -32_000)?; // ~32 MB
    c.pragma_update(None, "mmap_size", 256 * 1024 * 1024)?;
    Ok(())
}

fn migrate(c: &mut Connection) -> Result<()> {
    let current: usize = c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? as usize;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = c.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
        tracing::info!(version = i + 1, "migrazione del database applicata");
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn test_db() -> (tempfile::TempDir, Arc<Db>) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("mail.db")).unwrap();
    (dir, db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_migrates_and_has_fts5() {
        let (_dir, db) = test_db();
        db.read(|c| {
            let v: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
            assert_eq!(v as usize, MIGRATIONS.len());
            let mode: String = c.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
            assert_eq!(mode, "wal");
            Ok(())
        })
        .unwrap();
        db.write(|c| {
            c.execute("INSERT INTO search_index (rowid, subject, body) VALUES (1, 'Preventivo', 'ciao')", [])?;
            let n: i64 = c.query_row("SELECT count(*) FROM search_index WHERE search_index MATCH 'prev*'", [], |r| r.get(0))?;
            assert_eq!(n, 1);
            c.execute("DELETE FROM search_index WHERE rowid = 1", [])?;
            Ok(())
        })
        .unwrap();
    }
}
