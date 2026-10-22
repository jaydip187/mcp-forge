//! SQL connections, statement guards and the read-only query executor.
//!
//! Read-only enforcement is layered; each layer works on its own:
//! 1. the UI recommends a SELECT-only role and [`SqlExecutor::test_connection`]
//!    reports whether the configured role can write;
//! 2. the session is read-only with a server-side timeout ([`session`]);
//! 3. the statement must be a single read-only query, checked on the full
//!    AST ([`guard::check_read_only`]);
//! 4. functions that write or escape the database are denied ([`guard`]);
//! 5. results are capped by row limit, timeout and cell length.

mod error;
mod executor;
mod explore;
pub mod guard;
mod outcome;
mod policy;
mod pool;
pub mod session;
pub mod value;

pub use guard::StatementClass;
pub use session::{
    run_read_only, run_write, Bind, DbPool, Limits, RawResult, WriteOptions, WriteResult,
};

/// Replacement value for masked columns.
pub const MASKED: &str = "[masked]";

pub use error::SqlError;
pub use executor::SqlExecutor;
pub use outcome::{AuditDraft, SqlOutcome, SqlRun, WriteInfo, DRY_RUN_ARG};
pub use policy::{check_agent_sql, check_query_tool, gate, tool_input, tool_writes};
pub use pool::ConnectionReport;

#[cfg(test)]
#[path = "tests/lib.rs"]
mod tests;
