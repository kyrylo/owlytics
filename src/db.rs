use rusqlite::Connection;
use std::{
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
use tokio::sync::Semaphore;
pub const SCHEMA_VERSION: i64 = 17;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database worker failed")]
    Worker,
    #[error("database is busy; try again")]
    Busy,
    #[error("database schema is newer than this Owlytics version")]
    NewerSchema,
    #[error("database file could not be opened: {0}")]
    Filesystem(#[from] std::io::Error),
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

struct Readers {
    connections: Mutex<Vec<Connection>>,
    admission: Arc<Semaphore>,
}

/// A bounded writer queue and two independent WAL readers. No task waits on a
/// database mutex while occupying a blocking worker.
#[derive(Clone)]
pub struct Database {
    writer: mpsc::SyncSender<Job>,
    readers: Option<Arc<Readers>>,
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        #[cfg(unix)]
        if path.as_ref() != Path::new(":memory:") {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .mode(0o600)
                .open(path.as_ref())?;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        let path = path.as_ref();
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            PRAGMA cache_size = -4096;
            PRAGMA temp_store = FILE;
            PRAGMA mmap_size = 0;
            PRAGMA journal_size_limit = 16777216;
            PRAGMA secure_delete = ON;
        ",
        )?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::NewerSchema);
        }
        let upgrading = (version > 0 && version < SCHEMA_VERSION).then(std::time::Instant::now);
        if upgrading.is_some() {
            eprintln!("Upgrading SQLite schema {version} to {SCHEMA_VERSION}…");
        }
        if version == 0 {
            tx.execute_batch(
                "CREATE TABLE owner_sessions (
                token_hash TEXT PRIMARY KEY,
                csrf TEXT NOT NULL,
                credential_hash TEXT NOT NULL,
                expires_at INTEGER NOT NULL
            );
            CREATE TABLE login_limit (
                id INTEGER PRIMARY KEY CHECK(id=1),
                window_start INTEGER NOT NULL,
                attempts INTEGER NOT NULL
            );
            PRAGMA user_version = 1;",
            )?;
        }
        if version < 2 {
            tx.execute_batch(include_str!("schema.sql"))?;
        }
        if version < 3 {
            tx.execute_batch(include_str!("imports.sql"))?;
        }
        if version < 4 {
            tx.execute_batch(include_str!("rollups.sql"))?;
        }
        if version < 5 {
            tx.execute_batch(include_str!("indexes.sql"))?;
        }
        if version < 6 {
            tx.execute_batch(include_str!("sessions.sql"))?;
            let sources = tx
                .prepare("SELECT DISTINCT source FROM sessions")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            for source in sources {
                let name = crate::analytics::source_name(&source);
                if name != source {
                    tx.execute(
                        "UPDATE sessions SET source=?1 WHERE source=?2",
                        rusqlite::params![name, source],
                    )?;
                    tx.execute(
                        "UPDATE events SET source=?1 WHERE source=?2",
                        rusqlite::params![name, source],
                    )?;
                }
            }
            tx.execute_batch("DELETE FROM daily_reports WHERE dimension='source'; INSERT INTO daily_reports SELECT site_id,day,'source',source,COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day,source;")?;
        }
        if version < 7 {
            tx.execute_batch(include_str!("session_rollups.sql"))?;
        }
        if version < 8 {
            tx.execute_batch(include_str!("engagement.sql"))?;
        }
        if version < 9 {
            tx.execute_batch(include_str!("acquisition.sql"))?;
        }
        if version < 10 {
            tx.execute_batch(include_str!("import_groups.sql"))?;
        }
        if version < 11 {
            tx.execute_batch(include_str!("session_lookup.sql"))?;
        }
        if version < 12 {
            crate::location_upgrade::migrate(&tx)?;
        }
        if version < 13 {
            tx.execute_batch(include_str!("session_activity.sql"))?;
        }
        if version < 14 {
            tx.execute_batch(include_str!("visit_attributes.sql"))?;
        }
        if version < 15 {
            tx.execute_batch(include_str!("cross_day_reports.sql"))?;
        }
        if version < 16 {
            tx.execute_batch(include_str!("session_continuity.sql"))?;
        }
        if version < 17 {
            tx.execute_batch(include_str!("owner_settings.sql"))?;
        }
        tx.commit()?;
        if let Some(started) = upgrading {
            eprintln!(
                "SQLite schema upgrade complete ({:.1}s)",
                started.elapsed().as_secs_f64()
            );
        }
        let readers = if path == Path::new(":memory:") {
            None
        } else {
            let mut connections = Vec::new();
            for _ in 0..2 {
                let reader =
                    Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                reader.busy_timeout(Duration::from_secs(5))?;
                reader.execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-2048; PRAGMA temp_store=FILE; PRAGMA mmap_size=0;")?;
                connections.push(reader);
            }
            Some(Arc::new(Readers {
                connections: Mutex::new(connections),
                admission: Arc::new(Semaphore::new(2)),
            }))
        };
        let (writer, receiver) = mpsc::sync_channel::<Job>(32);
        std::thread::Builder::new()
            .name("sqlite-writer".into())
            .spawn(move || {
                for job in receiver {
                    // A failed caller must not take the database worker with it.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        job(&mut connection)
                    }));
                }
            })?;
        Ok(Self { writer, readers })
    }

    pub async fn call<F, T>(&self, operation: F) -> Result<T, Error>
    where
        F: FnOnce(&mut Connection) -> Result<T, rusqlite::Error> + Send + 'static,
        T: Send + 'static,
    {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.writer
            .try_send(Box::new(move |connection| {
                let _ = sender.send(operation(connection));
            }))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => Error::Busy,
                mpsc::TrySendError::Disconnected(_) => Error::Worker,
            })?;
        receiver
            .await
            .map_err(|_| Error::Worker)?
            .map_err(Error::from)
    }

    pub async fn read<F, T>(&self, operation: F) -> Result<T, Error>
    where
        F: FnOnce(&mut Connection) -> Result<T, rusqlite::Error> + Send + 'static,
        T: Send + 'static,
    {
        let Some(readers) = &self.readers else {
            return self.call(operation).await;
        };
        let permit = readers
            .admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let readers = readers.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut connection = readers
                .connections
                .lock()
                .map_err(|_| Error::Worker)?
                .pop()
                .ok_or(Error::Worker)?;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                operation(&mut connection)
            }));
            readers
                .connections
                .lock()
                .map_err(|_| Error::Worker)?
                .push(connection);
            result.map_err(|_| Error::Worker)?.map_err(Error::from)
        })
        .await
        .map_err(|_| Error::Worker)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writer_queue_is_bounded_and_recovers_after_a_panicking_caller() {
        use std::{
            future::Future,
            task::{Context, Waker},
        };
        let db = Database::open(":memory:").unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let clone = db.clone();
        let worker = tokio::spawn(async move {
            clone
                .call(move |_| {
                    let _ = started.send(());
                    blocked.recv().unwrap();
                    Ok(())
                })
                .await
        });
        ready.await.unwrap();
        let mut context = Context::from_waker(Waker::noop());
        let mut queued = Vec::new();
        for _ in 0..32 {
            let mut pending =
                Box::pin(db.call(|c| c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))));
            assert!(pending.as_mut().poll(&mut context).is_pending());
            queued.push(pending);
        }
        assert!(matches!(db.call(|_| Ok(())).await, Err(Error::Busy)));
        release.send(()).unwrap();
        worker.await.unwrap().unwrap();
        for pending in queued {
            assert_eq!(pending.await.unwrap(), 1);
        }
        assert!(matches!(
            db.call::<_, ()>(|_| panic!("caller failed")).await,
            Err(Error::Worker)
        ));
        assert_eq!(
            db.call(|c| c.query_row("SELECT 42", [], |r| r.get::<_, i64>(0)))
                .await
                .unwrap(),
            42
        );
    }

    #[tokio::test]
    async fn wal_reader_snapshot_does_not_block_the_writer() {
        let path = std::env::temp_dir().join(format!(
            "owlytics-wal-{}.sqlite",
            crate::auth::random_token().unwrap()
        ));
        let db = Database::open(&path).unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let clone = db.clone();
        let reader = tokio::spawn(async move {
            clone
                .read(move |c| {
                    let tx = c.transaction()?;
                    let before: i64 =
                        tx.query_row("SELECT COUNT(*) FROM sites", [], |r| r.get(0))?;
                    let _ = started.send(());
                    blocked.recv_timeout(Duration::from_secs(5)).unwrap();
                    let after: i64 =
                        tx.query_row("SELECT COUNT(*) FROM sites", [], |r| r.get(0))?;
                    assert_eq!((before, after), (0, 0));
                    tx.commit()
                })
                .await
        });
        ready.await.unwrap();
        let write = tokio::time::timeout(
            Duration::from_secs(2),
            db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])),
        )
        .await;
        release.send(()).unwrap();
        assert_eq!(write.unwrap().unwrap(), 1);
        reader.await.unwrap().unwrap();
        assert_eq!(
            db.read(|c| c.query_row("SELECT COUNT(*) FROM sites", [], |r| r.get::<_, i64>(0)))
                .await
                .unwrap(),
            1
        );
        assert!(
            db.read(|c| c.execute("DELETE FROM sites", []))
                .await
                .is_err()
        );
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn enforces_foreign_keys_and_transaction_rollback() {
        let db = Database::open(":memory:").unwrap();
        db.call(|c| c.execute_batch("CREATE TABLE parents(id INTEGER PRIMARY KEY); CREATE TABLE children(parent INTEGER REFERENCES parents(id));")).await.unwrap();
        assert!(
            db.call(|c| c.execute("INSERT INTO children VALUES (42)", []))
                .await
                .is_err()
        );
        db.call(|c| {
            let tx = c.transaction()?;
            tx.execute("INSERT INTO parents VALUES (42)", [])?;
            // Dropping a transaction must roll it back.
            Ok(())
        })
        .await
        .unwrap();
        let count = db
            .call(|c| c.query_row("SELECT COUNT(*) FROM parents", [], |r| r.get::<_, i64>(0)))
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}
