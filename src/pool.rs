//! Return-time disposal for connections whose transaction outcome is uncertain.
//!
//! Mutable borrowing remains available for trusted migration/transaction callers.
//! Such callers can replace a raw connection: this wrapper is not an isolation
//! boundary against arbitrary in-process Rust code.

use std::ops::{Deref, DerefMut};
use std::path::Path;

use r2d2::ManageConnection;
use rusqlite::{Connection, OpenFlags};

/// Owns the configured manager, including its shared-memory keepalive.
#[derive(Debug)]
pub struct SqliteConnectionManager(r2d2_sqlite::SqliteConnectionManager);

impl SqliteConnectionManager {
    pub fn new(manager: r2d2_sqlite::SqliteConnectionManager) -> Self {
        Self(manager)
    }

    pub fn file(path: impl AsRef<Path>) -> Self {
        Self::new(r2d2_sqlite::SqliteConnectionManager::file(path))
    }

    pub fn memory() -> Self {
        Self::new(r2d2_sqlite::SqliteConnectionManager::memory())
    }

    pub fn with_flags(self, flags: OpenFlags) -> Self {
        Self::new(self.0.with_flags(flags))
    }

    pub fn with_init<F>(self, init: F) -> Self
    where
        F: Fn(&mut Connection) -> rusqlite::Result<()> + Send + Sync + 'static,
    {
        Self::new(self.0.with_init(init))
    }
}

#[derive(Debug)]
pub struct ManagedConnection {
    connection: Connection,
    quarantined: bool,
}

impl ManagedConnection {
    /// Irreversible even if SQLite later reports autocommit: transaction state
    /// alone cannot establish whether an uncertain write committed.
    pub fn quarantine(&mut self) {
        self.quarantined = true;
    }
}

impl Deref for ManagedConnection {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.connection
    }
}

impl DerefMut for ManagedConnection {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}

impl ManageConnection for SqliteConnectionManager {
    type Connection = ManagedConnection;
    type Error = rusqlite::Error;

    fn connect(&self) -> rusqlite::Result<ManagedConnection> {
        self.0.connect().map(|connection| ManagedConnection {
            connection,
            quarantined: false,
        })
    }

    fn is_valid(&self, conn: &mut ManagedConnection) -> rusqlite::Result<()> {
        if conn.quarantined || !conn.is_autocommit() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        self.0.is_valid(&mut conn.connection)
    }

    fn has_broken(&self, conn: &mut ManagedConnection) -> bool {
        conn.quarantined || !conn.is_autocommit() || self.0.has_broken(&mut conn.connection)
    }
}

pub type PooledConn = r2d2::PooledConnection<SqliteConnectionManager>;

/// Owning projection for transport-free consumers of lazy connection iterators.
pub(crate) struct ConnectionLease(pub PooledConn);

impl Deref for ConnectionLease {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarantine_replaces_connection_but_preserves_memory_identity() {
        let manager = SqliteConnectionManager::new(
            r2d2_sqlite::SqliteConnectionManager::memory()
                .with_init(|c| c.execute_batch("PRAGMA foreign_keys=ON;")),
        );
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("pool");
        {
            let mut conn = pool.get().expect("first");
            conn.execute_batch("CREATE TABLE durable(n); INSERT INTO durable VALUES(9); CREATE TEMP TABLE marker(n);")
                .expect("fixture");
            conn.quarantine();
            assert!(conn.is_autocommit());
        }
        let conn = pool.get().expect("replacement");
        assert!(conn.prepare("SELECT * FROM marker").is_err());
        assert_eq!(
            conn.query_row("SELECT n FROM durable", [], |r| r.get::<_, i64>(0))
                .expect("same memory DB"),
            9
        );
        assert_eq!(
            conn.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                .expect("initializer"),
            1
        );
    }

    #[test]
    fn manager_preserves_flags_and_initialization_errors() {
        let tmp = tempfile::NamedTempFile::new().expect("temporary DB");
        Connection::open(tmp.path())
            .expect("fixture")
            .execute_batch("CREATE TABLE durable(n);")
            .expect("schema");
        let manager = SqliteConnectionManager::new(
            r2d2_sqlite::SqliteConnectionManager::file(tmp.path())
                .with_flags(OpenFlags::SQLITE_OPEN_READ_ONLY),
        );
        let mut conn = manager.connect().expect("read only connection");
        assert!(conn.execute("INSERT INTO durable VALUES(1)", []).is_err());
        assert!(manager.is_valid(&mut conn).is_ok());
        conn.quarantine();
        assert!(manager.is_valid(&mut conn).is_err());
        assert!(manager.has_broken(&mut conn));
        let failing =
            SqliteConnectionManager::memory().with_init(|_| Err(rusqlite::Error::InvalidQuery));
        assert!(matches!(
            failing.connect(),
            Err(rusqlite::Error::InvalidQuery)
        ));
    }
}
