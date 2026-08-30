use std::collections::BTreeSet;

use lenso_postgres_kit::{OwnedPostgres, SchemaOperator, SetupOutcome, UpgradeOutcome};
use sqlx::{AssertSqlSafe, Executor as _};
use uuid::Uuid;

use crate::{KnowledgeBaseOperator, schema, storage};

#[tokio::test]
#[allow(clippy::too_many_lines)]
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
    let legacy_setup = SchemaOperator::connect(
        &database_url,
        schema::legacy_schema_plan(schema_name.clone()).unwrap(),
    )
    .await
    .unwrap()
    .setup()
    .await
    .unwrap();
    assert_eq!(
        legacy_setup,
        SetupOutcome::Created {
            version: 1,
            applied: 1
        }
    );
    let upgraded = KnowledgeBaseOperator::upgrade(&database_url, &schema_name)
        .await
        .unwrap();
    assert_eq!(
        upgraded,
        UpgradeOutcome::Applied {
            from: 1,
            to: 2,
            applied: 1
        }
    );
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

    let unpublished = storage::update_draft(
        &postgres,
        "knowledge-base-api",
        "update-unpublished",
        &[7],
        "org_acme",
        created.article_id,
        "usr_second_editor",
        3,
        Some("Use the unreleased recovery flow"),
        None,
    )
    .await
    .unwrap();
    let draft = storage::get_draft(&postgres, "org_acme", created.article_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(draft.title, "Use the unreleased recovery flow");
    assert_eq!(draft.revision, unpublished.revision);
    assert_eq!(draft.updated_by, "usr_second_editor");
    assert_eq!(draft.latest_publication_revision, Some(2));
    assert_eq!(draft.latest_published_article_revision, Some(3));
    assert_eq!(draft.latest_published_by.as_deref(), Some("usr_publisher"));
    assert!(draft.latest_published_at.is_some());
    assert!(
        storage::get_draft(&postgres, "org_other", created.article_id)
            .await
            .unwrap()
            .is_none()
    );

    let second = storage::create_draft(
        &postgres,
        "knowledge-base-api",
        "create-second",
        &[8],
        "org_acme",
        "usr_editor",
        "configure-passkeys",
        "Configure passkeys",
        "Configure a passkey in account settings.",
    )
    .await
    .unwrap();
    let third = storage::create_draft(
        &postgres,
        "knowledge-base-api",
        "create-third",
        &[9],
        "org_acme",
        "usr_editor",
        "contact-support",
        "Contact support",
        "Open a support case.",
    )
    .await
    .unwrap();
    let other_organization = storage::create_draft(
        &postgres,
        "knowledge-base-api",
        "create-other-organization",
        &[10],
        "org_other",
        "usr_other_editor",
        "private-runbook",
        "Private runbook",
        "Organization-local body.",
    )
    .await
    .unwrap();

    let first_page = storage::list_articles(&postgres, "org_acme", None, 2)
        .await
        .unwrap();
    assert_eq!(first_page.len(), 2);
    let cursor =
        storage::decode_article_cursor(&storage::encode_article_cursor(first_page.last().unwrap()))
            .unwrap();
    let edited_page_one = first_page.first().unwrap();
    storage::update_draft(
        &postgres,
        "knowledge-base-api",
        "update-page-one",
        &[11],
        "org_acme",
        edited_page_one.article_id,
        "usr_editor",
        edited_page_one.revision,
        Some("Edited without moving the list cursor"),
        None,
    )
    .await
    .unwrap();
    let second_page = storage::list_articles(&postgres, "org_acme", Some(&cursor), 2)
        .await
        .unwrap();
    assert_eq!(second_page.len(), 1);
    let article_ids = first_page
        .iter()
        .chain(&second_page)
        .map(|record| record.article_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        article_ids,
        BTreeSet::from([created.article_id, second.article_id, third.article_id])
    );
    let other_articles = storage::list_articles(&postgres, "org_other", None, 10)
        .await
        .unwrap();
    assert_eq!(other_articles.len(), 1);
    assert_eq!(other_articles[0].article_id, other_organization.article_id);

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
