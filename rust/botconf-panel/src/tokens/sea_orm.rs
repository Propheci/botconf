//! Handles kept in a SQL table, with SeaORM, so that long buttons keep
//! working after a restart.

use futures::future::BoxFuture;
use sea_orm::{
    ConnectionTrait, DatabaseConnection,
    sea_query::{
        Alias, ColumnDef, Expr, ExprTrait, OnConflict, Query, Table, TableCreateStatement,
    },
};

use super::TokenStore;
use botconf::StorageError;

/// The table used unless another one is given.
pub const DEFAULT_TOKENS_TABLE: &str = "panel_tokens";

const KEY: &str = "key";
const ACTION: &str = "action";
const EXPIRES_AT: &str = "expires_at";

/// The statement creating the table, for the program's migrations: `key`
/// (string, primary key), `action` (text) and `expires_at` (big integer,
/// milliseconds since the Unix epoch).
pub fn create_tokens_table(table: &str) -> TableCreateStatement {
    Table::create()
        .table(Alias::new(table))
        .if_not_exists()
        .col(
            ColumnDef::new(Alias::new(KEY))
                .string()
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(Alias::new(ACTION)).text().not_null())
        .col(
            ColumnDef::new(Alias::new(EXPIRES_AT))
                .big_integer()
                .not_null(),
        )
        .to_owned()
}

/// Keeps the handles in a table (see [`create_tokens_table`]).
#[derive(Clone, Debug)]
pub struct SeaOrmTokenStore {
    db: DatabaseConnection,
    table: String,
}

impl SeaOrmTokenStore {
    /// Uses the [`DEFAULT_TOKENS_TABLE`].
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            db,
            table: DEFAULT_TOKENS_TABLE.to_string(),
        }
    }

    #[must_use]
    pub fn with_table(mut self, table: impl Into<String>) -> Self {
        self.table = table.into();
        self
    }

    fn table(&self) -> Alias {
        Alias::new(&self.table)
    }
}

impl TokenStore for SeaOrmTokenStore {
    fn put<'a>(
        &'a self,
        key: &'a str,
        action: &'a str,
        now_ms: i64,
        expires_at_ms: i64,
    ) -> BoxFuture<'a, Result<bool, StorageError>> {
        Box::pin(async move {
            // One statement: insert, or replace the row when it holds the same
            // action or has expired. A live row holding another action is
            // left alone, and no row is affected.
            let column = |name: &str| Expr::col((self.table(), Alias::new(name)));
            let excluded = |name: &str| Expr::col((Alias::new("excluded"), Alias::new(name)));
            let insert = Query::insert()
                .into_table(self.table())
                .columns([Alias::new(KEY), Alias::new(ACTION), Alias::new(EXPIRES_AT)])
                .values_panic([key.into(), action.into(), expires_at_ms.into()])
                .on_conflict(
                    OnConflict::column(Alias::new(KEY))
                        .update_columns([Alias::new(ACTION), Alias::new(EXPIRES_AT)])
                        .action_and_where(
                            column(ACTION)
                                .eq(excluded(ACTION))
                                .or(column(EXPIRES_AT).lte(now_ms)),
                        )
                        .to_owned(),
                )
                .to_owned();
            let result = self.db.execute(&insert).await?;
            Ok(result.rows_affected() == 1)
        })
    }

    fn get<'a>(
        &'a self,
        key: &'a str,
        now_ms: i64,
    ) -> BoxFuture<'a, Result<Option<String>, StorageError>> {
        Box::pin(async move {
            let query = Query::select()
                .column(Alias::new(ACTION))
                .from(self.table())
                .and_where(Expr::col(Alias::new(KEY)).eq(key))
                .and_where(Expr::col(Alias::new(EXPIRES_AT)).gt(now_ms))
                .to_owned();
            let row = self.db.query_one(&query).await?;
            Ok(row.map(|row| row.try_get("", ACTION)).transpose()?)
        })
    }

    fn purge_expired(&self, now_ms: i64) -> BoxFuture<'_, Result<u64, StorageError>> {
        Box::pin(async move {
            let delete = Query::delete()
                .from_table(self.table())
                .and_where(Expr::col(Alias::new(EXPIRES_AT)).lte(now_ms))
                .to_owned();
            Ok(self.db.execute(&delete).await?.rows_affected())
        })
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::Database;

    use super::*;

    #[tokio::test]
    async fn sea_orm_token_store_renews_refuses_collisions_and_purges() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute(&create_tokens_table("tokens")).await.unwrap();
        let store = SeaOrmTokenStore::new(db).with_table("tokens");

        assert!(store.put("~a", "o:h", 0, 100).await.unwrap());
        // The same action: renewed.
        assert!(store.put("~a", "o:h", 50, 200).await.unwrap());
        assert_eq!(store.get("~a", 150).await.unwrap().as_deref(), Some("o:h"));
        // Another action under a live key: refused, nothing changes.
        assert!(!store.put("~a", "x", 60, 300).await.unwrap());
        assert_eq!(store.get("~a", 150).await.unwrap().as_deref(), Some("o:h"));
        // Over an expired entry: replaced.
        assert!(store.put("~a", "x", 200, 300).await.unwrap());
        assert_eq!(store.get("~a", 250).await.unwrap().as_deref(), Some("x"));
        // After expiry: gone, then purged.
        assert_eq!(store.get("~a", 300).await.unwrap(), None);
        assert!(store.put("~b", "R", 0, 400).await.unwrap());
        assert_eq!(store.purge_expired(300).await.unwrap(), 1);
        assert_eq!(store.get("~b", 300).await.unwrap().as_deref(), Some("R"));
    }
}
