//! Static checks on SQL before it reaches a database: placeholder binding and
//! the read-only AST guard (layers 3 and 4 of the read-only model).

mod check;
mod classify;
mod deny;
mod masks;
mod placeholders;
mod tables;

use forge_core::Driver;
use sqlparser::dialect::{Dialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect};

pub use check::{check_read_only, check_statement, CheckedQuery};
pub use classify::StatementClass;
pub use masks::{check_masks, masked_columns};
pub use placeholders::{bind_placeholders, BoundSql};
pub use tables::{is_system_table, table_allowed};

fn dialect(driver: Driver) -> Box<dyn Dialect> {
    match driver {
        Driver::Sqlite => Box::new(SQLiteDialect {}),
        Driver::Postgres => Box::new(PostgreSqlDialect {}),
        Driver::Mysql => Box::new(MySqlDialect {}),
    }
}

#[cfg(test)]
#[path = "tests/guard.rs"]
mod tests;
