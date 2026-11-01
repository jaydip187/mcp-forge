//! Static checks on SQL before it reaches a database: placeholder binding and
//! the read-only AST guard (layers 3 and 4 of the read-only model).

mod check;
mod classify;
mod deny;
mod files;
mod masks;
mod placeholders;
mod tables;

use forge_core::Driver;
use sqlparser::dialect::{
    ClickHouseDialect, Dialect, DuckDbDialect, GenericDialect, MsSqlDialect, MySqlDialect,
    PostgreSqlDialect, SQLiteDialect,
};

pub use check::{check_read_only, check_statement, CheckedQuery};
pub use classify::StatementClass;
pub use files::{check_file_reads, file_table_name, FILE_FUNCTIONS};
pub use masks::{check_masks, masked_columns};
pub use placeholders::{bind_placeholders, BoundSql};
pub use tables::{is_system_table, table_allowed};

fn dialect(driver: Driver) -> Box<dyn Dialect> {
    match driver {
        Driver::Sqlite => Box::new(SQLiteDialect {}),
        Driver::Postgres => Box::new(PostgreSqlDialect {}),
        Driver::Mysql => Box::new(MySqlDialect {}),
        Driver::Duckdb => Box::new(DuckDbDialect {}),
        Driver::Mssql => Box::new(MsSqlDialect {}),
        Driver::Clickhouse => Box::new(ClickHouseDialect {}),
        // NoSQL connections run no SQL; anything sent there parses generically.
        Driver::Mongodb | Driver::Redis | Driver::Dynamodb | Driver::Elasticsearch => {
            Box::new(GenericDialect {})
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/guard.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/unit/guard_dialects.rs"]
mod dialect_tests;
