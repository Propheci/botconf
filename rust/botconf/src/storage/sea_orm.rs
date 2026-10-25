//! Overrides kept in a SQL table, with SeaORM.
//!
//! Actors (`updated_by`) are text. Tables made by earlier versions declare the
//! column `bigint`:
//! - on SQLite they keep working: loading casts the column to text, so the
//!   numbers stored before read back as strings (a migration to a `text` column
//!   is still worth doing, so that what is stored is what was written);
//! - on Postgres they need a migration, since a `bigint` column rejects text:
//!   `ALTER TABLE settings ALTER COLUMN updated_by TYPE text USING
//!   updated_by::text`.

use chrono::Utc;
use futures::future::BoxFuture;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, TransactionTrait,
    sea_query::{
        Alias, ColumnDef, Expr, ExprTrait, OnConflict, Query, Table, TableCreateStatement,
    },
};

use super::{Storage, StorageError, StoredOverride};

/// The table used unless another one is given.
pub const DEFAULT_TABLE: &str = "settings";

const KEY: &str = "key";
const VALUE: &str = "value";
const UPDATED_BY: &str = "updated_by";
const UPDATED_AT: &str = "updated_at";

/// The statement creating the table, for the program's migrations:
/// `key` (primary key), `value` (JSON text), `updated_by` and `updated_at`.
pub fn create_table(table: &str) -> TableCreateStatement {
    Table::create()
        .table(Alias::new(table))
        .if_not_exists()
        .col(
            ColumnDef::new(Alias::new(KEY))
                .string()
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(Alias::new(VALUE)).text().not_null())
        .col(ColumnDef::new(Alias::new(UPDATED_BY)).text().null())
        .col(
            ColumnDef::new(Alias::new(UPDATED_AT))
                .timestamp_with_time_zone()
                .not_null(),
        )
        .to_owned()
}

/// Keeps the overrides in a table (see [`create_table`]).
#[derive(Clone, Debug)]
pub struct SeaOrmStorage {
    db: DatabaseConnection,
    table: String,
}

impl SeaOrmStorage {
    /// Uses the [`DEFAULT_TABLE`].
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            db,
            table: DEFAULT_TABLE.to_string(),
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

impl Storage for SeaOrmStorage {
    fn load(&self) -> BoxFuture<'_, Result<Vec<StoredOverride>, StorageError>> {
        Box::pin(async move {
            let query = Query::select()
                .columns([Alias::new(KEY), Alias::new(VALUE)])
                // Cast: tables created before actors were strings declare the
                // column `bigint`, where SQLite keeps a numeric-looking actor
                // (any Telegram user id) as an INTEGER, which does not decode as
                // a string.
                .expr_as(
                    Expr::col(Alias::new(UPDATED_BY)).cast_as(Alias::new("TEXT")),
                    Alias::new(UPDATED_BY),
                )
                .from(self.table())
                .order_by(Alias::new(KEY), sea_orm::sea_query::Order::Asc)
                .to_owned();
            let rows = self.db.query_all(&query).await?;
            rows.into_iter()
                .map(|row| {
                    Ok(StoredOverride {
                        key: row.try_get("", KEY)?,
                        value: row.try_get("", VALUE)?,
                        by: row.try_get("", UPDATED_BY)?,
                    })
                })
                .collect::<Result<_, sea_orm::DbErr>>()
                .map_err(Into::into)
        })
    }

    fn write<'a>(
        &'a self,
        deleted: &'a [String],
        upserted: Option<&'a StoredOverride>,
    ) -> BoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async move {
            let txn = self.db.begin().await?;
            if !deleted.is_empty() {
                let delete = Query::delete()
                    .from_table(self.table())
                    .and_where(Expr::col(Alias::new(KEY)).is_in(deleted.iter().cloned()))
                    .to_owned();
                txn.execute(&delete).await?;
            }
            if let Some(stored) = upserted {
                let insert = Query::insert()
                    .into_table(self.table())
                    .columns([
                        Alias::new(KEY),
                        Alias::new(VALUE),
                        Alias::new(UPDATED_BY),
                        Alias::new(UPDATED_AT),
                    ])
                    .values_panic([
                        stored.key.clone().into(),
                        stored.value.clone().into(),
                        stored.by.clone().into(),
                        Utc::now().into(),
                    ])
                    .on_conflict(
                        OnConflict::column(Alias::new(KEY))
                            .update_columns([
                                Alias::new(VALUE),
                                Alias::new(UPDATED_BY),
                                Alias::new(UPDATED_AT),
                            ])
                            .to_owned(),
                    )
                    .to_owned();
                txn.execute(&insert).await?;
            }
            txn.commit().await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use sea_orm::Database;

    use super::*;

    #[tokio::test]
    async fn stores_and_replaces_overrides() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute(&create_table("overrides")).await.unwrap();
        let storage = SeaOrmStorage::new(db).with_table("overrides");

        let stored = |key: &str, value: &str, by: Option<String>| StoredOverride {
            key: key.into(),
            value: value.into(),
            by,
        };
        storage
            .write(&[], Some(&stored("b", "1", None)))
            .await
            .unwrap();
        storage
            .write(&[], Some(&stored("a", "\"x\"", Some("7".into()))))
            .await
            .unwrap();
        storage
            .write(&[], Some(&stored("b", "2", Some("8".into()))))
            .await
            .unwrap();
        assert_eq!(
            storage.load().await.unwrap(),
            [
                stored("a", "\"x\"", Some("7".into())),
                stored("b", "2", Some("8".into()))
            ]
        );

        storage
            .write(&["a".into(), "missing".into()], None)
            .await
            .unwrap();
        assert_eq!(
            storage.load().await.unwrap(),
            [stored("b", "2", Some("8".into()))]
        );

        // Actors are opaque: not every platform id is a number.
        storage
            .write(&[], Some(&stored("c", "3", Some("123@lid".into()))))
            .await
            .unwrap();
        assert_eq!(
            storage.load().await.unwrap()[1],
            stored("c", "3", Some("123@lid".into()))
        );
    }

    /// Tables created before actors were strings declare `updated_by` as
    /// `bigint`. SQLite stores a numeric-looking text in such a column as an
    /// INTEGER, which must still load as a string.
    #[tokio::test]
    async fn old_bigint_tables_still_load() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE overrides (key varchar NOT NULL PRIMARY KEY, value text NOT NULL, \
             updated_by bigint NULL, updated_at timestamp_with_timezone_text NOT NULL)",
        )
        .await
        .unwrap();
        db.execute_unprepared(
            "INSERT INTO overrides VALUES ('a', '1', 42, '2026-10-25'), ('b', '2', '123@lid', \
             '2026-10-25'), ('c', '3', NULL, '2026-10-25')",
        )
        .await
        .unwrap();

        let storage = SeaOrmStorage::new(db).with_table("overrides");
        let actors: Vec<_> = storage
            .load()
            .await
            .unwrap()
            .into_iter()
            .map(|stored| stored.by)
            .collect();
        assert_eq!(actors, [Some("42".into()), Some("123@lid".into()), None]);
    }
}
