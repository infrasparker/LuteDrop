use rusqlite::Error as SqliteError;
use rusqlite::ErrorCode;

/// Typed errors for the database layer. Introduced so callers can branch on a
/// variant (e.g. a UNIQUE collision on bulk import) instead of matching a
/// formatted error string. Collapses to `String` at the command/action
/// boundary via `Display`.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// A UNIQUE / PRIMARY KEY / other constraint was violated.
    #[error("constraint violation")]
    Constraint,
    /// A required row was not found.
    #[error("{0}")]
    NotFound(String),
    /// Any other SQLite failure.
    #[error("database error: {0}")]
    Sqlite(#[from] SqliteError),
}

impl From<DbError> for String {
    fn from(e: DbError) -> String {
        e.to_string()
    }
}

impl DbError {
    /// Classify a rusqlite error: constraint violations become `Constraint`,
    /// everything else stays `Sqlite`.
    pub(crate) fn from_sqlite(e: SqliteError) -> Self {
        match &e {
            SqliteError::SqliteFailure(f, _) if f.code == ErrorCode::ConstraintViolation => {
                DbError::Constraint
            }
            _ => DbError::Sqlite(e),
        }
    }
}
