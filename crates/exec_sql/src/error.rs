//! Executor errors and DSN redaction.

use forge_core::ValidationErrors;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SqlError {
    #[error("{0}")]
    InvalidArgs(ValidationErrors),
    /// The statement failed the read-only check (layers 3â€“4).
    #[error("query rejected: {0}")]
    Rejected(String),
    /// Missing connection, unresolvable secret, malformed DSN.
    #[error("{0}")]
    Config(String),
    #[error("could not connect to the database: {0}")]
    Connect(String),
    #[error("the query did not finish within {0} ms")]
    Timeout(u32),
    #[error("database error: {0}")]
    Database(String),
    /// A write changed more rows than the connection allows; rolled back.
    #[error(
        "the statement would change {}{affected} rows, more than this connection allows ({max}); nothing was changed",
        if *at_least { "at least " } else { "" }
    )]
    TooManyRows {
        affected: u64,
        max: u32,
        at_least: bool,
    },
}

impl SqlError {
    pub(crate) fn connect(e: sqlx::Error) -> Self {
        SqlError::Connect(e.to_string())
    }

    pub(crate) fn redact(self, r: &Redactor) -> Self {
        match self {
            SqlError::Rejected(m) => SqlError::Rejected(r.apply(&m)),
            SqlError::Config(m) => SqlError::Config(r.apply(&m)),
            SqlError::Connect(m) => SqlError::Connect(r.apply(&m)),
            SqlError::Database(m) => SqlError::Database(r.apply(&m)),
            other => other,
        }
    }
}

impl From<sqlx::Error> for SqlError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(d) = &e {
            // Postgres query_canceled; MySQL 3024 max_execution_time exceeded.
            if d.code().as_deref() == Some("57014")
                || d.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
                    .is_some_and(|m| m.number() == 3024)
            {
                return SqlError::Timeout(0);
            }
            return SqlError::Database(d.message().to_owned());
        }
        match e {
            sqlx::Error::PoolTimedOut
            | sqlx::Error::Io(_)
            | sqlx::Error::Tls(_)
            | sqlx::Error::Configuration(_) => SqlError::Connect(e.to_string()),
            other => SqlError::Database(other.to_string()),
        }
    }
}

/// Strips the DSN and its password from messages before they leave the executor.
pub(crate) struct Redactor(Vec<String>);

impl Redactor {
    pub(crate) fn new(dsn: &str) -> Self {
        let mut secrets = vec![dsn.to_owned()];
        if let Ok(u) = url::Url::parse(dsn) {
            if let Some(pw) = u.password() {
                secrets.push(pw.to_owned());
                if let Ok(decoded) = urldecode(pw) {
                    secrets.push(decoded);
                }
            }
        }
        // libpq key=value form: `password=...`.
        for part in dsn.split_whitespace() {
            if let Some(pw) = part.strip_prefix("password=") {
                secrets.push(pw.trim_matches('\'').to_owned());
            }
        }
        secrets.retain(|s| s.len() >= 3);
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        Self(secrets)
    }

    pub(crate) fn apply(&self, msg: &str) -> String {
        self.0
            .iter()
            .fold(msg.to_owned(), |m, s| m.replace(s.as_str(), "***"))
    }
}

pub(crate) fn urldecode(s: &str) -> Result<String, std::string::FromUtf8Error> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out)
}
