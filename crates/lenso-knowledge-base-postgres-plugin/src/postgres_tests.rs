use lenso_postgres_kit::OwnedPostgres;
use sqlx::{AssertSqlSafe, Executor as _};
use uuid::Uuid;

use crate::{KnowledgeBaseOperator, schema, storage};

#[tokio::test]
async fn draft_publication_idempotency_and_restart_are_durable() {
    let Ok(database_url) = std::env::var("LENSO_KNOWLEDGE_BASE_TEST_DATABASE_URL") else {
        return;
    };
    let database_name = database_url
        .split('?')
        .next()
        .and_then(|value| value.rsplit('/').next())
        .unwrap_or_default();
    assert!(
        database_name.starts_with("lenso_knowledge_base_test"),
        "acceptance requires a dedicated lenso_knowledge_base_test database"
    );
    let schema_name = format!("knowledge_base_test_{}", Uuid::new_v4().simple());
    KnowledgeBaseOperator::setup(&database_url, &schema_name)
        .await
        .unwrap();
    let postgres = OwnedPostgres::prepare(
        &database_url,
        schema::schema_plan(schema_name.clone()).unwrap(),
    )
    .await
    .unwrap();

    let created = storage::create_draft(
        &postgres,
        "knowledge-base-api",
        "create-1",
        &[1],
        "org_acme",
        "usr_editor",
        "reset-password",
        "Reset a password",
        "Use the recovery flow.",
    )
    .await
    .unwrap();
    let replayed = storage::create_draft(
        &postgres,
        "knowledge-base-api",
        "create-1",
        &[1],
        "org_acme",
        "usr_editor",
        "reset-password",
        "Reset a password",
        "Use the recovery flow.",
    )
    .await
    .unwrap();
    assert_eq!(created, replayed);

    let updated = storage::update_draft(
        &postgres,
        "knowledge-base-api",
        "update-1",
        &[2],
        "org_acme",
        created.article_id,
        "usr_editor",
        1,
        Some("Reset your password"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(updated.revision, 2);
    let conflict = storage::update_draft(
        &postgres,
        "knowledge-base-api",
        "update-2",
        &[3],
        "org_acme",
        created.article_id,
        "usr_editor",
        1,
        Some("Stale edit"),
        None,
    )
    .await;
    assert!(matches!(
        conflict,
        Err(storage::StorageError::Domain(
            storage::DomainFailure::RevisionConflict
        ))
    ));

    let published = storage::publish_article(
        &postgres,
        "knowledge-base-api",
        "publish-1",
        &[4],
        "org_acme",
        created.article_id,
        "usr_publisher",
        2,
    )
    .await
    .unwrap();
    assert!(!published.index_synchronized);

    let newer_draft = storage::update_draft(
        &postgres,
        "knowledge-base-api",
        "update-newer-publication",
        &[5],
        "org_acme",
        created.article_id,
        "usr_editor",
        2,
        Some("Use the newest recovery flow"),
        Some("The new search term is passkey."),
    )
    .await
    .unwrap();
    let newer_publication = storage::publish_article(
        &postgres,
        "knowledge-base-api",
        "publish-newer",
        &[6],
        "org_acme",
        created.article_id,
        "usr_publisher",
        newer_draft.revision,
    )
    .await
    .unwrap();
    storage::mark_publication_indexed(&postgres, "knowledge-base-api", "publish-newer")
        .await
        .unwrap();

    let stale_replay = storage::publish_article(
        &postgres,
        "knowledge-base-api",
        "publish-1",
        &[4],
        "org_acme",
        created.article_id,
        "usr_publisher",
        2,
    )
    .await
    .unwrap();
    assert_eq!(stale_replay.record, published.record);
    assert!(!stale_replay.index_synchronized);
    let (index_lock, publication_selected_for_retry) =
        storage::lock_latest_publication(&postgres, "org_acme", created.article_id)
            .await
            .unwrap();
    assert_eq!(publication_selected_for_retry, newer_publication.record);
    index_lock.commit().await.unwrap();
    storage::mark_publication_indexed(&postgres, "knowledge-base-api", "publish-1")
        .await
        .unwrap();
    postgres.pool().close().await;

    let restarted = OwnedPostgres::prepare(
        &database_url,
        schema::schema_plan(schema_name.clone()).unwrap(),
    )
    .await
    .unwrap();
    let replayed_publication = storage::publish_article(
        &restarted,
        "knowledge-base-api",
        "publish-1",
        &[4],
        "org_acme",
        created.article_id,
        "usr_publisher",
        2,
    )
    .await
    .unwrap();
    assert_eq!(replayed_publication.record, published.record);
    assert!(replayed_publication.index_synchronized);
    let fetched =
        storage::get_published_article(&restarted, "org_acme", &created.article_id.to_string())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(fetched.title, "Use the newest recovery flow");
    assert_eq!(fetched.article_revision, 3);
    assert_eq!(fetched.publication_revision, 2);

    restarted.pool().close().await;
    let cleanup = sqlx::PgPool::connect(&database_url).await.unwrap();
    cleanup
        .execute(AssertSqlSafe(format!(
            "DROP SCHEMA \"{schema_name}\" CASCADE"
        )))
        .await
        .unwrap();
    cleanup.close().await;
}
