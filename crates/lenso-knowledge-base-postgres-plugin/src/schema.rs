use lenso_postgres_kit::{Migration, PlanError, SchemaPlan, sql_migrations};

const MIGRATIONS: &[Migration] = sql_migrations![
    (
        1,
        "create-knowledge-base",
        "migrations/001_create_knowledge_base.sql",
    ),
    (
        2,
        "index-author-article-list",
        "migrations/002_index_author_article_list.sql",
    ),
];

#[cfg(all(test, feature = "postgres-acceptance"))]
const LEGACY_MIGRATIONS: &[Migration] = sql_migrations![(
    1,
    "create-knowledge-base",
    "migrations/001_create_knowledge_base.sql",
)];

pub(crate) fn schema_plan(schema: impl Into<std::sync::Arc<str>>) -> Result<SchemaPlan, PlanError> {
    SchemaPlan::new(schema, MIGRATIONS)
}

#[cfg(all(test, feature = "postgres-acceptance"))]
pub(crate) fn legacy_schema_plan(
    schema: impl Into<std::sync::Arc<str>>,
) -> Result<SchemaPlan, PlanError> {
    SchemaPlan::new(schema, LEGACY_MIGRATIONS)
}
