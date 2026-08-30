use std::collections::BTreeSet;

use lenso_postgres_kit::OwnedPostgres;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sqlx::{Postgres, Row, Transaction};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

const CREATE_DRAFT: &str = "create_draft";
const UPDATE_DRAFT: &str = "update_draft";
const PUBLISH_ARTICLE: &str = "publish_article";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct DraftRecord {
    pub article_id: Uuid,
    pub organization_id: String,
    pub slug: String,
    pub title: String,
    pub body_markdown: String,
    pub revision: i64,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PublishedRecord {
    pub article_id: Uuid,
    pub organization_id: String,
    pub slug: String,
    pub title: String,
    pub body_markdown: String,
    pub article_revision: i64,
    pub publication_revision: i64,
    pub published_by: String,
    pub published_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PublicationOutcome {
    pub record: PublishedRecord,
    pub index_synchronized: bool,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub(crate) enum DomainFailure {
    #[error("article not found")]
    ArticleNotFound,
    #[error("idempotency key was reused for a different command")]
    IdempotencyConflict,
    #[error("article revision conflict")]
    RevisionConflict,
    #[error("article slug conflict")]
    SlugConflict,
}

#[derive(Debug, Error)]
pub(crate) enum StorageError {
    #[error("Knowledge Base domain failure: {0:?}")]
    Domain(#[from] DomainFailure),
    #[error("Knowledge Base PostgreSQL failure: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Knowledge Base receipt serialization failure: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Knowledge Base timestamp formatting failure: {0}")]
    Time(#[from] time::error::Format),
    #[error("Knowledge Base receipt is incomplete after command serialization")]
    IncompleteReceipt,
}

enum ReceiptClaim<T> {
    Claimed,
    Replayed {
        response: T,
        index_synchronized: bool,
    },
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_draft(
    postgres: &OwnedPostgres,
    caller: &str,
    idempotency_key: &str,
    request_hash: &[u8],
    organization_id: &str,
    actor: &str,
    slug: &str,
    title: &str,
    body_markdown: &str,
) -> Result<DraftRecord, StorageError> {
    let mut transaction = postgres.pool().begin().await?;
    match claim_receipt::<DraftRecord>(
        &mut transaction,
        caller,
        CREATE_DRAFT,
        idempotency_key,
        request_hash,
    )
    .await?
    {
        ReceiptClaim::Replayed { response, .. } => {
            transaction.commit().await?;
            return Ok(response);
        }
        ReceiptClaim::Claimed => {}
    }

    let article_id = Uuid::new_v4();
    let inserted = sqlx::query(
        "INSERT INTO knowledge_base_articles \
         (organization_id,article_id,slug,current_revision,created_by) \
         VALUES ($1,$2,$3,1,$4) RETURNING created_at,updated_at",
    )
    .bind(organization_id)
    .bind(article_id)
    .bind(slug)
    .bind(actor)
    .fetch_one(&mut *transaction)
    .await;
    let inserted = match inserted {
        Ok(row) => row,
        Err(error) if is_unique_violation(&error) => {
            return Err(DomainFailure::SlugConflict.into());
        }
        Err(error) => return Err(error.into()),
    };
    sqlx::query(
        "INSERT INTO knowledge_base_article_revisions \
         (organization_id,article_id,revision,slug,title,body_markdown,created_by) \
         VALUES ($1,$2,1,$3,$4,$5,$6)",
    )
    .bind(organization_id)
    .bind(article_id)
    .bind(slug)
    .bind(title)
    .bind(body_markdown)
    .bind(actor)
    .execute(&mut *transaction)
    .await?;

    let record = DraftRecord {
        article_id,
        organization_id: organization_id.to_owned(),
        slug: slug.to_owned(),
        title: title.to_owned(),
        body_markdown: body_markdown.to_owned(),
        revision: 1,
        created_by: actor.to_owned(),
        created_at: timestamp(&inserted, "created_at")?,
        updated_at: timestamp(&inserted, "updated_at")?,
    };
    finish_receipt(
        &mut transaction,
        caller,
        CREATE_DRAFT,
        idempotency_key,
        &record,
        false,
    )
    .await?;
    transaction.commit().await?;
    Ok(record)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_draft(
    postgres: &OwnedPostgres,
    caller: &str,
    idempotency_key: &str,
    request_hash: &[u8],
    organization_id: &str,
    article_id: Uuid,
    actor: &str,
    expected_revision: i64,
    title: Option<&str>,
    body_markdown: Option<&str>,
) -> Result<DraftRecord, StorageError> {
    let mut transaction = postgres.pool().begin().await?;
    match claim_receipt::<DraftRecord>(
        &mut transaction,
        caller,
        UPDATE_DRAFT,
        idempotency_key,
        request_hash,
    )
    .await?
    {
        ReceiptClaim::Replayed { response, .. } => {
            transaction.commit().await?;
            return Ok(response);
        }
        ReceiptClaim::Claimed => {}
    }

    let existing = sqlx::query(
        "SELECT a.slug,a.current_revision,a.created_by,a.created_at, \
                r.title,r.body_markdown \
         FROM knowledge_base_articles a \
         JOIN knowledge_base_article_revisions r \
           ON r.organization_id=a.organization_id \
          AND r.article_id=a.article_id \
          AND r.revision=a.current_revision \
         WHERE a.organization_id=$1 AND a.article_id=$2 \
         FOR UPDATE OF a",
    )
    .bind(organization_id)
    .bind(article_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(DomainFailure::ArticleNotFound)?;
    let current_revision: i64 = existing.try_get("current_revision")?;
    if current_revision != expected_revision {
        return Err(DomainFailure::RevisionConflict.into());
    }
    let next_revision = current_revision + 1;
    let next_slug = existing.try_get::<&str, _>("slug")?;
    let next_title = title.unwrap_or(existing.try_get::<&str, _>("title")?);
    let next_body = body_markdown.unwrap_or(existing.try_get::<&str, _>("body_markdown")?);

    let updated = sqlx::query(
        "UPDATE knowledge_base_articles \
         SET slug=$3,current_revision=$4,updated_at=transaction_timestamp() \
         WHERE organization_id=$1 AND article_id=$2 \
         RETURNING updated_at",
    )
    .bind(organization_id)
    .bind(article_id)
    .bind(next_slug)
    .bind(next_revision)
    .fetch_one(&mut *transaction)
    .await;
    let updated = match updated {
        Ok(row) => row,
        Err(error) if is_unique_violation(&error) => {
            return Err(DomainFailure::SlugConflict.into());
        }
        Err(error) => return Err(error.into()),
    };
    sqlx::query(
        "INSERT INTO knowledge_base_article_revisions \
         (organization_id,article_id,revision,slug,title,body_markdown,created_by) \
         VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(organization_id)
    .bind(article_id)
    .bind(next_revision)
    .bind(next_slug)
    .bind(next_title)
    .bind(next_body)
    .bind(actor)
    .execute(&mut *transaction)
    .await?;

    let record = DraftRecord {
        article_id,
        organization_id: organization_id.to_owned(),
        slug: next_slug.to_owned(),
        title: next_title.to_owned(),
        body_markdown: next_body.to_owned(),
        revision: next_revision,
        created_by: existing.try_get("created_by")?,
        created_at: timestamp(&existing, "created_at")?,
        updated_at: timestamp(&updated, "updated_at")?,
    };
    finish_receipt(
        &mut transaction,
        caller,
        UPDATE_DRAFT,
        idempotency_key,
        &record,
        false,
    )
    .await?;
    transaction.commit().await?;
    Ok(record)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn publish_article(
    postgres: &OwnedPostgres,
    caller: &str,
    idempotency_key: &str,
    request_hash: &[u8],
    organization_id: &str,
    article_id: Uuid,
    actor: &str,
    expected_revision: i64,
) -> Result<PublicationOutcome, StorageError> {
    let mut transaction = postgres.pool().begin().await?;
    match claim_receipt::<PublishedRecord>(
        &mut transaction,
        caller,
        PUBLISH_ARTICLE,
        idempotency_key,
        request_hash,
    )
    .await?
    {
        ReceiptClaim::Replayed {
            response,
            index_synchronized,
        } => {
            transaction.commit().await?;
            return Ok(PublicationOutcome {
                record: response,
                index_synchronized,
            });
        }
        ReceiptClaim::Claimed => {}
    }

    lock_article_publication(&mut transaction, organization_id, article_id).await?;

    let current = sqlx::query(
        "SELECT a.current_revision,a.latest_publication_revision, \
                r.slug,r.title,r.body_markdown \
         FROM knowledge_base_articles a \
         JOIN knowledge_base_article_revisions r \
           ON r.organization_id=a.organization_id \
          AND r.article_id=a.article_id \
          AND r.revision=a.current_revision \
         WHERE a.organization_id=$1 AND a.article_id=$2 \
         FOR UPDATE OF a",
    )
    .bind(organization_id)
    .bind(article_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(DomainFailure::ArticleNotFound)?;
    let current_revision: i64 = current.try_get("current_revision")?;
    if current_revision != expected_revision {
        return Err(DomainFailure::RevisionConflict.into());
    }
    let latest_publication: Option<i64> = current.try_get("latest_publication_revision")?;
    let publication_revision = latest_publication.unwrap_or(0) + 1;
    let publication = sqlx::query(
        "INSERT INTO knowledge_base_publications \
         (organization_id,article_id,publication_revision,article_revision,published_by) \
         VALUES ($1,$2,$3,$4,$5) RETURNING published_at",
    )
    .bind(organization_id)
    .bind(article_id)
    .bind(publication_revision)
    .bind(current_revision)
    .bind(actor)
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE knowledge_base_articles \
         SET latest_publication_revision=$3,updated_at=transaction_timestamp() \
         WHERE organization_id=$1 AND article_id=$2",
    )
    .bind(organization_id)
    .bind(article_id)
    .bind(publication_revision)
    .execute(&mut *transaction)
    .await?;

    let record = PublishedRecord {
        article_id,
        organization_id: organization_id.to_owned(),
        slug: current.try_get("slug")?,
        title: current.try_get("title")?,
        body_markdown: current.try_get("body_markdown")?,
        article_revision: current_revision,
        publication_revision,
        published_by: actor.to_owned(),
        published_at: timestamp(&publication, "published_at")?,
    };
    finish_receipt(
        &mut transaction,
        caller,
        PUBLISH_ARTICLE,
        idempotency_key,
        &record,
        false,
    )
    .await?;
    transaction.commit().await?;
    Ok(PublicationOutcome {
        record,
        index_synchronized: false,
    })
}

/// Locks one article's publication/index order and returns its current publication.
///
/// The transaction must remain open until the corresponding Search upsert has
/// completed. New publications take the same transaction-scoped advisory lock,
/// so an older retry cannot overwrite a newer article in the Search index.
pub(crate) async fn lock_latest_publication<'a>(
    postgres: &'a OwnedPostgres,
    organization_id: &str,
    article_id: Uuid,
) -> Result<(Transaction<'a, Postgres>, PublishedRecord), StorageError> {
    let mut transaction = postgres.pool().begin().await?;
    lock_article_publication(&mut transaction, organization_id, article_id).await?;
    let row = sqlx::query(
        "SELECT a.organization_id,a.article_id,r.slug,r.title,r.body_markdown, \
                p.article_revision,p.publication_revision,p.published_by,p.published_at \
         FROM knowledge_base_articles a \
         JOIN knowledge_base_publications p \
           ON p.organization_id=a.organization_id \
          AND p.article_id=a.article_id \
          AND p.publication_revision=a.latest_publication_revision \
         JOIN knowledge_base_article_revisions r \
           ON r.organization_id=p.organization_id \
          AND r.article_id=p.article_id \
          AND r.revision=p.article_revision \
         WHERE a.organization_id=$1 AND a.article_id=$2",
    )
    .bind(organization_id)
    .bind(article_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(DomainFailure::ArticleNotFound)?;
    let record = published_from_row(&row)?;
    Ok((transaction, record))
}

async fn lock_article_publication(
    transaction: &mut Transaction<'_, Postgres>,
    organization_id: &str,
    article_id: Uuid,
) -> Result<(), StorageError> {
    let lock_key = format!("{}:{organization_id}:{article_id}", organization_id.len());
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(lock_key)
        .fetch_one(&mut **transaction)
        .await?;
    Ok(())
}

pub(crate) async fn mark_publication_indexed(
    postgres: &OwnedPostgres,
    caller: &str,
    idempotency_key: &str,
) -> Result<(), StorageError> {
    let changed = sqlx::query(
        "UPDATE knowledge_base_command_receipts \
         SET index_synchronized=true \
         WHERE caller_instance=$1 AND operation=$2 AND idempotency_key=$3 \
           AND response_json IS NOT NULL",
    )
    .bind(caller)
    .bind(PUBLISH_ARTICLE)
    .bind(idempotency_key)
    .execute(postgres.pool())
    .await?;
    if changed.rows_affected() != 1 {
        return Err(StorageError::IncompleteReceipt);
    }
    Ok(())
}

pub(crate) async fn get_published_article(
    postgres: &OwnedPostgres,
    organization_id: &str,
    article_ref: &str,
) -> Result<Option<PublishedRecord>, StorageError> {
    let row = sqlx::query(
        "SELECT a.organization_id,a.article_id,r.slug,r.title,r.body_markdown, \
                p.article_revision,p.publication_revision,p.published_by,p.published_at \
         FROM knowledge_base_articles a \
         JOIN knowledge_base_publications p \
           ON p.organization_id=a.organization_id \
          AND p.article_id=a.article_id \
          AND p.publication_revision=a.latest_publication_revision \
         JOIN knowledge_base_article_revisions r \
           ON r.organization_id=p.organization_id \
          AND r.article_id=p.article_id \
          AND r.revision=p.article_revision \
         WHERE a.organization_id=$1 \
           AND (a.article_id::text=$2 OR a.slug=$2)",
    )
    .bind(organization_id)
    .bind(article_ref)
    .fetch_optional(postgres.pool())
    .await?;
    row.as_ref().map(published_from_row).transpose()
}

pub(crate) async fn published_references(
    postgres: &OwnedPostgres,
    organization_id: &str,
    source_ids: &[String],
) -> Result<Vec<PublishedRecord>, StorageError> {
    let mut seen = BTreeSet::new();
    let mut records = Vec::with_capacity(source_ids.len());
    for source_id in source_ids {
        if !seen.insert(source_id) {
            continue;
        }
        if Uuid::parse_str(source_id).is_err() {
            continue;
        }
        if let Some(record) = get_published_article(postgres, organization_id, source_id).await? {
            records.push(record);
        }
    }
    Ok(records)
}

async fn claim_receipt<T: DeserializeOwned>(
    transaction: &mut Transaction<'_, Postgres>,
    caller: &str,
    operation: &str,
    idempotency_key: &str,
    request_hash: &[u8],
) -> Result<ReceiptClaim<T>, StorageError> {
    let inserted = sqlx::query(
        "INSERT INTO knowledge_base_command_receipts \
         (caller_instance,operation,idempotency_key,request_hash) \
         VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING",
    )
    .bind(caller)
    .bind(operation)
    .bind(idempotency_key)
    .bind(request_hash)
    .execute(&mut **transaction)
    .await?;
    if inserted.rows_affected() == 1 {
        return Ok(ReceiptClaim::Claimed);
    }
    let row = sqlx::query(
        "SELECT request_hash,response_json,index_synchronized \
         FROM knowledge_base_command_receipts \
         WHERE caller_instance=$1 AND operation=$2 AND idempotency_key=$3 \
         FOR UPDATE",
    )
    .bind(caller)
    .bind(operation)
    .bind(idempotency_key)
    .fetch_one(&mut **transaction)
    .await?;
    let stored_hash: Vec<u8> = row.try_get("request_hash")?;
    if stored_hash != request_hash {
        return Err(DomainFailure::IdempotencyConflict.into());
    }
    let value: Option<serde_json::Value> = row.try_get("response_json")?;
    let value = value.ok_or(StorageError::IncompleteReceipt)?;
    Ok(ReceiptClaim::Replayed {
        response: serde_json::from_value(value)?,
        index_synchronized: row.try_get("index_synchronized")?,
    })
}

async fn finish_receipt<T: Serialize>(
    transaction: &mut Transaction<'_, Postgres>,
    caller: &str,
    operation: &str,
    idempotency_key: &str,
    response: &T,
    index_synchronized: bool,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE knowledge_base_command_receipts \
         SET response_json=$4,index_synchronized=$5,completed_at=transaction_timestamp() \
         WHERE caller_instance=$1 AND operation=$2 AND idempotency_key=$3",
    )
    .bind(caller)
    .bind(operation)
    .bind(idempotency_key)
    .bind(serde_json::to_value(response)?)
    .bind(index_synchronized)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn published_from_row(row: &sqlx::postgres::PgRow) -> Result<PublishedRecord, StorageError> {
    Ok(PublishedRecord {
        article_id: row.try_get("article_id")?,
        organization_id: row.try_get("organization_id")?,
        slug: row.try_get("slug")?,
        title: row.try_get("title")?,
        body_markdown: row.try_get("body_markdown")?,
        article_revision: row.try_get("article_revision")?,
        publication_revision: row.try_get("publication_revision")?,
        published_by: row.try_get("published_by")?,
        published_at: timestamp(row, "published_at")?,
    })
}

fn timestamp(row: &sqlx::postgres::PgRow, column: &str) -> Result<String, StorageError> {
    Ok(row.try_get::<OffsetDateTime, _>(column)?.format(&Rfc3339)?)
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if database.code().as_deref() == Some("23505"))
}
