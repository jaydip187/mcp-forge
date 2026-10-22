//! Statement classes, from read to admin.

use sqlparser::ast::Statement;

/// What a statement does, from least to most privileged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatementClass {
    #[default]
    Read,
    /// INSERT, UPDATE, DELETE, MERGE (and locking reads).
    Write,
    /// CREATE, ALTER, DROP, TRUNCATE, … (and SELECT … INTO).
    Ddl,
    /// Everything else: GRANT, SET, COPY, VACUUM, PRAGMA, CALL, … Never allowed.
    Admin,
}

impl StatementClass {
    pub fn as_str(self) -> &'static str {
        match self {
            StatementClass::Read => "read",
            StatementClass::Write => "write",
            StatementClass::Ddl => "ddl",
            StatementClass::Admin => "admin",
        }
    }
}

pub(super) fn classify_one(s: &Statement) -> StatementClass {
    use StatementClass::*;
    match s {
        Statement::Query(_) => Read,
        Statement::Insert { .. }
        | Statement::Update { .. }
        | Statement::Delete { .. }
        | Statement::Merge { .. } => Write,
        Statement::CreateTable { .. }
        | Statement::CreateIndex { .. }
        | Statement::CreateView { .. }
        | Statement::CreateVirtualTable { .. }
        | Statement::CreateSchema { .. }
        | Statement::CreateSequence { .. }
        | Statement::CreateType { .. }
        | Statement::CreateDomain { .. }
        | Statement::CreateTrigger { .. }
        | Statement::CreateFunction { .. }
        | Statement::CreateProcedure { .. }
        | Statement::AlterTable { .. }
        | Statement::AlterIndex { .. }
        | Statement::AlterView { .. }
        | Statement::AlterType { .. }
        | Statement::AlterSchema { .. }
        | Statement::Drop { .. }
        | Statement::DropFunction { .. }
        | Statement::DropProcedure { .. }
        | Statement::DropTrigger { .. }
        | Statement::DropDomain { .. }
        | Statement::Truncate { .. }
        | Statement::RenameTable { .. }
        | Statement::Comment { .. } => Ddl,
        _ => Admin,
    }
}
