//! PostgreSQL-backed Knowledge Base behavior with source-owned authorization.

mod operator;
#[cfg(all(test, feature = "postgres-acceptance"))]
mod postgres_tests;
mod schema;
mod storage;

use std::{cell::RefCell, collections::BTreeSet, fmt, rc::Rc, time::Duration};

use lenso::prelude::*;
use lenso_auth_sdk::{
    ActorAssertion, ActorAssertionVerifier, ActorProjectionError, AssertionClock, TypedActor,
};
use lenso_capability_access_control as access;
use lenso_capability_access_control::{
    AccessControlInvocationError, CheckPermissionRequest, CheckPermissionRequestScope,
};
use lenso_capability_knowledge_base as knowledge;
use lenso_capability_knowledge_base::{
    CreateDraftError, CreateDraftRequest, CreateDraftResponse, GetDraftError, GetDraftRequest,
    GetDraftResponse, GetPublishedArticleError, GetPublishedArticleRequest,
    GetPublishedArticleResponse, ListArticlesError, ListArticlesRequest, ListArticlesResponse,
    ListArticlesResponseArticlesItem, PublishArticleError, PublishArticleRequest,
    PublishArticleResponse, SearchPublishedArticlesError, SearchPublishedArticlesRequest,
    SearchPublishedArticlesResponse, SearchPublishedArticlesResponseArticlesItem, UpdateDraftError,
    UpdateDraftRequest, UpdateDraftResponse,
};
use lenso_capability_organization_membership as membership;
use lenso_capability_organization_membership::{
    CheckMembershipRequest, OrganizationMembershipInvocationError,
};
use lenso_capability_search as search;
use lenso_capability_search::{QueryReferencesError, SearchInvocationError, SearchRequest};
use lenso_capability_search_index as search_index;
use lenso_capability_search_index::{
    SearchIndexUpsertDocumentInvocationError, UpsertDocumentRequest,
};
use lenso_capability_secrets as secrets;
use lenso_capability_secrets::{ResolveRequest, SecretsInvocationError};
use lenso_kernel::{PluginDependencies, RuntimeFailure};
use lenso_postgres_kit::OwnedPostgres;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::storage::{DomainFailure, StorageError};

pub use operator::{KnowledgeBaseOperator, KnowledgeBaseOperatorError};

const DEPENDENCY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CALLERS: usize = 64;
const MAX_ID_BYTES: usize = 512;
const MAX_SLUG_BYTES: usize = 160;
const MAX_TITLE_BYTES: usize = 300;
const MAX_BODY_BYTES: usize = 512 * 1024;
const MAX_IDEMPOTENCY_BYTES: usize = 200;
const MAX_QUERY_BYTES: usize = 512;
const SEARCH_SOURCE_KIND: &str = "knowledge-base-article";

const ARTICLES_CREATE: &str = "knowledge-base.articles.create";
const ARTICLES_EDIT: &str = "knowledge-base.articles.edit";
const ARTICLES_PUBLISH: &str = "knowledge-base.articles.publish";
const ARTICLES_READ: &str = "knowledge-base.articles.read";
const ARTICLES_SEARCH: &str = "knowledge-base.articles.search";

/// Immutable configuration for one Knowledge Base Plugin Instance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeBaseConfig {
    schema: String,
    database_url_secret: String,
    auth_issuer: String,
    auth_assertion_public_key: String,
    business_callers: Vec<String>,
    #[serde(default)]
    public_read_grants: Vec<PublicReadGrant>,
}

/// One exact public surface allowed to read published content for one Organization.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicReadGrant {
    caller_instance: String,
    organization_id: String,
}

impl PublicReadGrant {
    pub fn new(caller_instance: impl Into<String>, organization_id: impl Into<String>) -> Self {
        Self {
            caller_instance: caller_instance.into(),
            organization_id: organization_id.into(),
        }
    }
}

impl KnowledgeBaseConfig {
    pub fn new(
        schema: impl Into<String>,
        database_url_secret: impl Into<String>,
        auth_issuer: impl Into<String>,
        auth_assertion_public_key: impl Into<String>,
        business_callers: Vec<String>,
    ) -> Result<Self, KnowledgeBaseConfigError> {
        let value = Self {
            schema: schema.into(),
            database_url_secret: database_url_secret.into(),
            auth_issuer: auth_issuer.into(),
            auth_assertion_public_key: auth_assertion_public_key.into(),
            business_callers,
            public_read_grants: Vec::new(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn with_public_read_grants(
        mut self,
        grants: Vec<PublicReadGrant>,
    ) -> Result<Self, KnowledgeBaseConfigError> {
        self.public_read_grants = grants;
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<(), KnowledgeBaseConfigError> {
        schema::schema_plan(self.schema.clone())
            .map_err(|_| KnowledgeBaseConfigError::InvalidSchema)?;
        if !valid_secret_reference(&self.database_url_secret) {
            return Err(KnowledgeBaseConfigError::InvalidSecretReference);
        }
        if !valid_identifier(&self.auth_issuer, 256) {
            return Err(KnowledgeBaseConfigError::InvalidAuthIssuer);
        }
        ActorAssertionVerifier::from_public_key_base64(
            self.auth_issuer.clone(),
            &self.auth_assertion_public_key,
        )
        .map_err(|_| KnowledgeBaseConfigError::InvalidAuthPublicKey)?;
        if !valid_callers(&self.business_callers) {
            return Err(KnowledgeBaseConfigError::InvalidBusinessCallers);
        }
        if self.public_read_grants.len() > MAX_CALLERS
            || self.public_read_grants.iter().any(|grant| {
                !valid_identifier(&grant.caller_instance, MAX_ID_BYTES)
                    || !valid_identifier(&grant.organization_id, MAX_ID_BYTES)
            })
            || self
                .public_read_grants
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != self.public_read_grants.len()
        {
            return Err(KnowledgeBaseConfigError::InvalidPublicReadGrants);
        }
        Ok(())
    }

    fn verifier(&self) -> Result<ActorAssertionVerifier, RuntimeFailure> {
        ActorAssertionVerifier::from_public_key_base64(
            self.auth_issuer.clone(),
            &self.auth_assertion_public_key,
        )
        .map_err(|_| RuntimeFailure::InvalidResolvedPlan {
            detail: "Knowledge Base Auth verification key is invalid".to_owned(),
        })
    }
}

/// Invalid immutable Knowledge Base configuration.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum KnowledgeBaseConfigError {
    #[error("invalid owned PostgreSQL schema")]
    InvalidSchema,
    #[error("invalid database URL secret reference")]
    InvalidSecretReference,
    #[error("invalid Auth issuer")]
    InvalidAuthIssuer,
    #[error("invalid Auth assertion public key")]
    InvalidAuthPublicKey,
    #[error("business_callers must contain 1 to 64 unique exact Instance keys")]
    InvalidBusinessCallers,
    #[error(
        "public_read_grants must contain at most 64 unique exact caller and Organization pairs"
    )]
    InvalidPublicReadGrants,
}

fn validate_config(config: &KnowledgeBaseConfig) -> Result<(), RuntimeFailure> {
    config
        .validate()
        .map_err(|error| RuntimeFailure::InvalidResolvedPlan {
            detail: format!("Knowledge Base configuration is invalid: {error}"),
        })
}

#[derive(Clone, Debug)]
struct PreparedKnowledgeBase {
    postgres: OwnedPostgres,
}

#[lenso::plugin(
    lifecycle,
    configuration_schema = "configuration.schema.json",
    validate = validate_config
)]
#[derive(Clone)]
struct PostgresKnowledgeBasePlugin {
    #[config]
    config: KnowledgeBaseConfig,
    secrets: Port<secrets::SecretsClient>,
    membership: Port<membership::OrganizationMembershipClient>,
    access: Port<access::AccessControlClient>,
    search: Port<search::SearchClient>,
    search_index: Port<search_index::SearchIndexClient>,
    prepared: Rc<RefCell<Option<PreparedKnowledgeBase>>>,
}

impl fmt::Debug for PostgresKnowledgeBasePlugin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostgresKnowledgeBasePlugin")
            .field("schema", &self.config.schema)
            .field("prepared", &self.prepared.borrow().is_some())
            .field("business_caller_count", &self.config.business_callers.len())
            .field(
                "public_read_grant_count",
                &self.config.public_read_grants.len(),
            )
            .finish_non_exhaustive()
    }
}

#[lenso::provides(knowledge::KnowledgeBase)]
impl PostgresKnowledgeBasePlugin {}

#[derive(Clone, Debug)]
struct Authorized {
    caller: String,
    actor: String,
}

#[derive(Debug)]
enum AuthorizationFailure {
    Unauthenticated,
    Forbidden,
    Runtime(RuntimeFailure),
}

impl PostgresKnowledgeBasePlugin {
    fn prepared(&self) -> Result<PreparedKnowledgeBase, RuntimeFailure> {
        self.prepared
            .borrow()
            .clone()
            .ok_or_else(|| RuntimeFailure::PluginFailure {
                detail: "Knowledge Base Plugin is not prepared".to_owned(),
            })
    }

    async fn create_draft(
        &self,
        context: Ctx,
        request: CreateDraftRequest,
    ) -> PluginResult<CreateDraftResponse, CreateDraftError> {
        let authorized = self
            .authorize(
                &context,
                knowledge::CREATE_DRAFT_OPERATION,
                &request.organization_id,
                ARTICLES_CREATE,
            )
            .await
            .map_err(map_create_authorization)?;
        if !valid_slug(&request.slug)
            || !valid_text(&request.title, MAX_TITLE_BYTES, false)
            || !valid_text(&request.body_markdown, MAX_BODY_BYTES, true)
            || !valid_idempotency_key(&request.idempotency_key)
        {
            return Err(PluginError::domain(CreateDraftError::InvalidRequest));
        }
        let request_hash = request_hash(&request).map_err(PluginError::runtime)?;
        let result = storage::create_draft(
            &self.prepared().map_err(PluginError::runtime)?.postgres,
            &authorized.caller,
            &request.idempotency_key,
            &request_hash,
            &request.organization_id,
            &authorized.actor,
            &request.slug,
            &request.title,
            &request.body_markdown,
        )
        .await;
        Ok(draft_create_response(map_storage(
            result,
            map_create_storage,
        )?))
    }

    async fn update_draft(
        &self,
        context: Ctx,
        request: UpdateDraftRequest,
    ) -> PluginResult<UpdateDraftResponse, UpdateDraftError> {
        let authorized = self
            .authorize(
                &context,
                knowledge::UPDATE_DRAFT_OPERATION,
                &request.organization_id,
                ARTICLES_EDIT,
            )
            .await
            .map_err(map_update_authorization)?;
        let article_id = Uuid::parse_str(&request.article_id)
            .map_err(|_| PluginError::domain(UpdateDraftError::InvalidRequest))?;
        let expected_revision = parse_revision(&request.expected_revision)
            .ok_or_else(|| PluginError::domain(UpdateDraftError::InvalidRequest))?;
        let title = patch_value(request.title.as_ref())
            .map_err(|()| PluginError::domain(UpdateDraftError::InvalidRequest))?;
        let body_markdown = patch_value(request.body_markdown.as_ref())
            .map_err(|()| PluginError::domain(UpdateDraftError::InvalidRequest))?;
        if title.is_none() && body_markdown.is_none()
            || title.is_some_and(|value| !valid_text(value, MAX_TITLE_BYTES, false))
            || body_markdown.is_some_and(|value| !valid_text(value, MAX_BODY_BYTES, true))
            || !valid_idempotency_key(&request.idempotency_key)
        {
            return Err(PluginError::domain(UpdateDraftError::InvalidRequest));
        }
        let request_hash = request_hash(&request).map_err(PluginError::runtime)?;
        let result = storage::update_draft(
            &self.prepared().map_err(PluginError::runtime)?.postgres,
            &authorized.caller,
            &request.idempotency_key,
            &request_hash,
            &request.organization_id,
            article_id,
            &authorized.actor,
            expected_revision,
            title,
            body_markdown,
        )
        .await;
        Ok(draft_update_response(map_storage(
            result,
            map_update_storage,
        )?))
    }

    async fn get_draft(
        &self,
        context: Ctx,
        request: GetDraftRequest,
    ) -> PluginResult<GetDraftResponse, GetDraftError> {
        self.authorize(
            &context,
            knowledge::GET_DRAFT_OPERATION,
            &request.organization_id,
            ARTICLES_EDIT,
        )
        .await
        .map_err(map_get_draft_authorization)?;
        let article_id = Uuid::parse_str(&request.article_id)
            .map_err(|_| PluginError::domain(GetDraftError::InvalidRequest))?;
        let record = storage::get_draft(
            &self.prepared().map_err(PluginError::runtime)?.postgres,
            &request.organization_id,
            article_id,
        )
        .await
        .map_err(|error| storage_runtime(&error))
        .map_err(PluginError::runtime)?
        .ok_or_else(|| PluginError::domain(GetDraftError::ArticleNotFound))?;
        Ok(draft_read_response(&record))
    }

    async fn list_articles(
        &self,
        context: Ctx,
        request: ListArticlesRequest,
    ) -> PluginResult<ListArticlesResponse, ListArticlesError> {
        self.authorize(
            &context,
            knowledge::LIST_ARTICLES_OPERATION,
            &request.organization_id,
            ARTICLES_EDIT,
        )
        .await
        .map_err(map_list_authorization)?;
        if !(1..=100).contains(&request.limit) {
            return Err(PluginError::domain(ListArticlesError::InvalidRequest));
        }
        let cursor = match request.cursor.as_deref() {
            Some(value) => Some(
                storage::decode_article_cursor(value)
                    .ok_or_else(|| PluginError::domain(ListArticlesError::InvalidRequest))?,
            ),
            None => None,
        };
        let mut records = storage::list_articles(
            &self.prepared().map_err(PluginError::runtime)?.postgres,
            &request.organization_id,
            cursor.as_ref(),
            request.limit + 1,
        )
        .await
        .map_err(|error| storage_runtime(&error))
        .map_err(PluginError::runtime)?;
        let page_size = usize::try_from(request.limit)
            .map_err(|_| PluginError::domain(ListArticlesError::InvalidRequest))?;
        let has_more = records.len() > page_size;
        if has_more {
            records.pop();
        }
        let next_cursor = has_more
            .then(|| records.last().map(storage::encode_article_cursor))
            .flatten();
        Ok(ListArticlesResponse {
            articles: records.iter().map(article_summary_response).collect(),
            next_cursor,
        })
    }

    async fn publish_article(
        &self,
        context: Ctx,
        request: PublishArticleRequest,
    ) -> PluginResult<PublishArticleResponse, PublishArticleError> {
        let authorized = self
            .authorize(
                &context,
                knowledge::PUBLISH_ARTICLE_OPERATION,
                &request.organization_id,
                ARTICLES_PUBLISH,
            )
            .await
            .map_err(map_publish_authorization)?;
        let article_id = Uuid::parse_str(&request.article_id)
            .map_err(|_| PluginError::domain(PublishArticleError::InvalidRequest))?;
        let expected_revision = parse_revision(&request.expected_revision)
            .ok_or_else(|| PluginError::domain(PublishArticleError::InvalidRequest))?;
        if !valid_idempotency_key(&request.idempotency_key) {
            return Err(PluginError::domain(PublishArticleError::InvalidRequest));
        }
        let request_hash = request_hash(&request).map_err(PluginError::runtime)?;
        let prepared = self.prepared().map_err(PluginError::runtime)?;
        let outcome = map_storage(
            storage::publish_article(
                &prepared.postgres,
                &authorized.caller,
                &request.idempotency_key,
                &request_hash,
                &request.organization_id,
                article_id,
                &authorized.actor,
                expected_revision,
            )
            .await,
            map_publish_storage,
        )?;
        if !outcome.index_synchronized {
            let (index_lock, current_publication) = storage::lock_latest_publication(
                &prepared.postgres,
                &request.organization_id,
                article_id,
            )
            .await
            .map_err(|error| storage_runtime(&error))
            .map_err(PluginError::runtime)?;
            self.synchronize_publication(&context, &current_publication)
                .await
                .map_err(PluginError::runtime)?;
            index_lock
                .commit()
                .await
                .map_err(|error| storage_runtime(&StorageError::Database(error)))
                .map_err(PluginError::runtime)?;
            storage::mark_publication_indexed(
                &prepared.postgres,
                &authorized.caller,
                &request.idempotency_key,
            )
            .await
            .map_err(|error| storage_runtime(&error))
            .map_err(PluginError::runtime)?;
        }
        Ok(published_response(&outcome.record))
    }

    async fn get_published_article(
        &self,
        context: Ctx,
        request: GetPublishedArticleRequest,
    ) -> PluginResult<GetPublishedArticleResponse, GetPublishedArticleError> {
        if !self.public_read_allowed(&context, &request.organization_id) {
            self.authorize(
                &context,
                knowledge::GET_PUBLISHED_ARTICLE_OPERATION,
                &request.organization_id,
                ARTICLES_READ,
            )
            .await
            .map_err(map_get_authorization)?;
        }
        if !valid_article_ref(&request.article_ref) {
            return Err(PluginError::domain(
                GetPublishedArticleError::InvalidRequest,
            ));
        }
        let record = storage::get_published_article(
            &self.prepared().map_err(PluginError::runtime)?.postgres,
            &request.organization_id,
            &request.article_ref,
        )
        .await
        .map_err(|error| storage_runtime(&error))
        .map_err(PluginError::runtime)?
        .ok_or_else(|| PluginError::domain(GetPublishedArticleError::ArticleNotFound))?;
        Ok(get_response(&record))
    }

    async fn search_published_articles(
        &self,
        context: Ctx,
        request: SearchPublishedArticlesRequest,
    ) -> PluginResult<SearchPublishedArticlesResponse, SearchPublishedArticlesError> {
        if !self.public_read_allowed(&context, &request.organization_id) {
            self.authorize(
                &context,
                knowledge::SEARCH_PUBLISHED_ARTICLES_OPERATION,
                &request.organization_id,
                ARTICLES_SEARCH,
            )
            .await
            .map_err(map_search_authorization)?;
        }
        let query = request.query.trim();
        if query.is_empty()
            || query.len() > MAX_QUERY_BYTES
            || query.chars().any(char::is_control)
            || !(1..=100).contains(&request.limit)
        {
            return Err(PluginError::domain(
                SearchPublishedArticlesError::InvalidQuery,
            ));
        }
        let found = self
            .search
            .query_references_with_context(
                context,
                SearchRequest {
                    scope_kind: "organization".to_owned(),
                    scope_id: request.organization_id.clone(),
                    query: query.to_owned(),
                    source_kinds: vec![SEARCH_SOURCE_KIND.to_owned()],
                    limit: request.limit,
                },
            )
            .await
            .map_err(map_search_dependency)?;
        let source_ids = found
            .references
            .into_iter()
            .filter(|reference| reference.source_kind == SEARCH_SOURCE_KIND)
            .map(|reference| reference.source_id)
            .collect::<Vec<_>>();
        let records = storage::published_references(
            &self.prepared().map_err(PluginError::runtime)?.postgres,
            &request.organization_id,
            &source_ids,
        )
        .await
        .map_err(|error| storage_runtime(&error))
        .map_err(PluginError::runtime)?;
        let articles = records.iter().map(search_item).collect();
        Ok(SearchPublishedArticlesResponse {
            articles,
            index_revision: found.index_revision.to_string(),
        })
    }

    async fn synchronize_publication(
        &self,
        context: &Ctx,
        record: &storage::PublishedRecord,
    ) -> Result<(), RuntimeFailure> {
        self.search_index
            .upsert_document_with_context(
                context.clone(),
                UpsertDocumentRequest {
                    scope_kind: "organization".to_owned(),
                    scope_id: record.organization_id.clone(),
                    source_kind: SEARCH_SOURCE_KIND.to_owned(),
                    source_id: record.article_id.to_string(),
                    search_text: format!("{}\n{}", record.title, record.body_markdown),
                },
            )
            .await
            .map(|_| ())
            .map_err(|error| match error {
                SearchIndexUpsertDocumentInvocationError::Runtime(error) => error,
                SearchIndexUpsertDocumentInvocationError::Domain(_) => {
                    RuntimeFailure::ProtocolViolation {
                        capability: search_index::CAPABILITY_ID,
                    }
                }
            })
    }

    fn public_read_allowed(&self, context: &Ctx, organization_id: &str) -> bool {
        context.caller_instance().is_some_and(|caller| {
            self.config.public_read_grants.iter().any(|grant| {
                grant.caller_instance == caller && grant.organization_id == organization_id
            })
        })
    }

    async fn authorize(
        &self,
        context: &Ctx,
        operation: &str,
        organization_id: &str,
        permission: &str,
    ) -> Result<Authorized, AuthorizationFailure> {
        let caller = context
            .caller_instance()
            .filter(|caller| {
                self.config
                    .business_callers
                    .iter()
                    .any(|allowed| allowed == *caller)
            })
            .map(ToOwned::to_owned)
            .ok_or(AuthorizationFailure::Forbidden)?;
        let actor = self
            .config
            .verifier()
            .map_err(AuthorizationFailure::Runtime)?
            .project_context::<KnowledgeBaseActor>(
                context,
                knowledge::CAPABILITY_ID,
                operation,
                &UtcClock,
            )
            .map_err(|_| AuthorizationFailure::Unauthenticated)?
            .subject;
        if !valid_identifier(organization_id, MAX_ID_BYTES)
            || !valid_identifier(&actor, MAX_ID_BYTES)
        {
            return Err(AuthorizationFailure::Forbidden);
        }
        let membership = self
            .membership
            .check_membership_with_context(
                context.clone(),
                CheckMembershipRequest {
                    organization_id: organization_id.to_owned(),
                    subject: actor.clone(),
                },
            )
            .await
            .map_err(|error| match error {
                OrganizationMembershipInvocationError::Runtime(error) => {
                    AuthorizationFailure::Runtime(error)
                }
                OrganizationMembershipInvocationError::Domain(_) => {
                    AuthorizationFailure::Runtime(RuntimeFailure::ProtocolViolation {
                        capability: membership::CAPABILITY_ID,
                    })
                }
            })?;
        if !membership.active {
            return Err(AuthorizationFailure::Forbidden);
        }
        let decision = self
            .access
            .check_permission_with_context(
                context.clone(),
                CheckPermissionRequest {
                    subject: actor.clone(),
                    scope: CheckPermissionRequestScope {
                        kind: "organization".to_owned(),
                        id: organization_id.to_owned(),
                    },
                    permission: permission.to_owned(),
                },
            )
            .await
            .map_err(|error| match error {
                AccessControlInvocationError::Runtime(error) => {
                    AuthorizationFailure::Runtime(error)
                }
                AccessControlInvocationError::Domain(_) => {
                    AuthorizationFailure::Runtime(RuntimeFailure::ProtocolViolation {
                        capability: access::CAPABILITY_ID,
                    })
                }
            })?;
        if !decision.allowed {
            return Err(AuthorizationFailure::Forbidden);
        }
        Ok(Authorized { caller, actor })
    }
}

impl Lifecycle for PostgresKnowledgeBasePlugin {
    async fn activate(&self, context: ActivateContext) -> Result<(), RuntimeFailure> {
        let database_url = resolve_secret(
            &self.secrets,
            context.dependencies(),
            context.cancellation(),
            &self.config.database_url_secret,
        )
        .await?;
        let postgres = OwnedPostgres::prepare(
            &database_url,
            schema::schema_plan(self.config.schema.clone()).map_err(|error| {
                RuntimeFailure::InvalidResolvedPlan {
                    detail: error.to_string(),
                }
            })?,
        )
        .await
        .map_err(|error| RuntimeFailure::PluginFailure {
            detail: error.to_string(),
        })?;
        self.prepared
            .borrow_mut()
            .replace(PreparedKnowledgeBase { postgres });
        Ok(())
    }

    async fn deactivate(&self, _context: DeactivateContext) -> Result<(), RuntimeFailure> {
        let prepared = self.prepared.borrow_mut().take();
        if let Some(prepared) = prepared {
            prepared.postgres.pool().close().await;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct KnowledgeBaseActor {
    subject: String,
}

impl TypedActor for KnowledgeBaseActor {
    fn from_assertion(assertion: &ActorAssertion) -> Result<Self, ActorProjectionError> {
        Ok(Self {
            subject: assertion.subject().to_owned(),
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct UtcClock;

impl AssertionClock for UtcClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

async fn resolve_secret(
    secrets: &secrets::SecretsClient,
    dependencies: &PluginDependencies,
    cancellation: lenso_kernel::CancellationToken,
    reference: &str,
) -> Result<Zeroizing<String>, RuntimeFailure> {
    let context = dependencies.invocation_context_after(DEPENDENCY_TIMEOUT, cancellation)?;
    secrets
        .resolve_with_context(
            context,
            ResolveRequest {
                reference: reference.to_owned(),
            },
        )
        .await
        .map(|response| Zeroizing::new(response.value))
        .map_err(|error| match error {
            SecretsInvocationError::Domain(_) => RuntimeFailure::PluginFailure {
                detail: format!("database URL secret `{reference}` was rejected"),
            },
            SecretsInvocationError::Runtime(error) => error,
        })
}

fn request_hash(request: &impl Serialize) -> Result<Vec<u8>, RuntimeFailure> {
    serde_json::to_vec(request)
        .map(|bytes| Sha256::digest(bytes).to_vec())
        .map_err(|error| RuntimeFailure::PluginFailure {
            detail: format!("failed to hash Knowledge Base command: {error}"),
        })
}

fn map_storage<T, E>(
    result: Result<T, StorageError>,
    map: impl FnOnce(DomainFailure) -> E,
) -> PluginResult<T, E> {
    match result {
        Ok(value) => Ok(value),
        Err(StorageError::Domain(error)) => Err(PluginError::domain(map(error))),
        Err(error) => Err(PluginError::runtime(storage_runtime(&error))),
    }
}

fn storage_runtime(error: &StorageError) -> RuntimeFailure {
    RuntimeFailure::PluginFailure {
        detail: error.to_string(),
    }
}

fn map_create_storage(failure: DomainFailure) -> CreateDraftError {
    match failure {
        DomainFailure::SlugConflict => CreateDraftError::SlugConflict,
        DomainFailure::IdempotencyConflict => CreateDraftError::IdempotencyConflict,
        DomainFailure::ArticleNotFound | DomainFailure::RevisionConflict => {
            CreateDraftError::InvalidRequest
        }
    }
}

fn map_update_storage(failure: DomainFailure) -> UpdateDraftError {
    match failure {
        DomainFailure::ArticleNotFound => UpdateDraftError::ArticleNotFound,
        DomainFailure::RevisionConflict => UpdateDraftError::RevisionConflict,
        DomainFailure::SlugConflict => UpdateDraftError::SlugConflict,
        DomainFailure::IdempotencyConflict => UpdateDraftError::IdempotencyConflict,
    }
}

fn map_publish_storage(failure: DomainFailure) -> PublishArticleError {
    match failure {
        DomainFailure::ArticleNotFound => PublishArticleError::ArticleNotFound,
        DomainFailure::RevisionConflict => PublishArticleError::RevisionConflict,
        DomainFailure::IdempotencyConflict => PublishArticleError::IdempotencyConflict,
        DomainFailure::SlugConflict => PublishArticleError::InvalidRequest,
    }
}

fn map_search_dependency(
    error: SearchInvocationError,
) -> PluginError<SearchPublishedArticlesError> {
    match error {
        SearchInvocationError::Domain(QueryReferencesError::InvalidQuery) => {
            PluginError::domain(SearchPublishedArticlesError::InvalidQuery)
        }
        SearchInvocationError::Domain(_) => {
            PluginError::runtime(RuntimeFailure::ProtocolViolation {
                capability: search::CAPABILITY_ID,
            })
        }
        SearchInvocationError::Runtime(error) => PluginError::runtime(error),
    }
}

macro_rules! authorization_mapper {
    ($name:ident, $error:ty) => {
        fn $name(failure: AuthorizationFailure) -> PluginError<$error> {
            match failure {
                AuthorizationFailure::Unauthenticated => {
                    PluginError::domain(<$error>::Unauthenticated)
                }
                AuthorizationFailure::Forbidden => PluginError::domain(<$error>::Forbidden),
                AuthorizationFailure::Runtime(error) => PluginError::runtime(error),
            }
        }
    };
}

authorization_mapper!(map_create_authorization, CreateDraftError);
authorization_mapper!(map_update_authorization, UpdateDraftError);
authorization_mapper!(map_get_draft_authorization, GetDraftError);
authorization_mapper!(map_list_authorization, ListArticlesError);
authorization_mapper!(map_publish_authorization, PublishArticleError);
authorization_mapper!(map_get_authorization, GetPublishedArticleError);
authorization_mapper!(map_search_authorization, SearchPublishedArticlesError);

fn draft_create_response(record: storage::DraftRecord) -> CreateDraftResponse {
    CreateDraftResponse {
        article_id: record.article_id.to_string(),
        organization_id: record.organization_id,
        slug: record.slug,
        title: record.title,
        body_markdown: record.body_markdown,
        revision: record.revision.to_string(),
        created_by: record.created_by,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

fn draft_update_response(record: storage::DraftRecord) -> UpdateDraftResponse {
    UpdateDraftResponse {
        article_id: record.article_id.to_string(),
        organization_id: record.organization_id,
        slug: record.slug,
        title: record.title,
        body_markdown: record.body_markdown,
        revision: record.revision.to_string(),
        created_by: record.created_by,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

fn draft_read_response(record: &storage::DraftViewRecord) -> GetDraftResponse {
    GetDraftResponse {
        article_id: record.article_id.to_string(),
        organization_id: record.organization_id.clone(),
        slug: record.slug.clone(),
        title: record.title.clone(),
        body_markdown: record.body_markdown.clone(),
        revision: record.revision.to_string(),
        created_by: record.created_by.clone(),
        updated_by: record.updated_by.clone(),
        created_at: record.created_at.clone(),
        updated_at: record.updated_at.clone(),
        latest_publication_revision: record
            .latest_publication_revision
            .map(|value| value.to_string()),
        latest_published_article_revision: record
            .latest_published_article_revision
            .map(|value| value.to_string()),
        latest_published_by: record.latest_published_by.clone(),
        latest_published_at: record.latest_published_at.clone(),
    }
}

fn article_summary_response(
    record: &storage::ArticleSummaryRecord,
) -> ListArticlesResponseArticlesItem {
    ListArticlesResponseArticlesItem {
        article_id: record.article_id.to_string(),
        slug: record.slug.clone(),
        title: record.title.clone(),
        revision: record.revision.to_string(),
        created_by: record.created_by.clone(),
        updated_by: record.updated_by.clone(),
        created_at: record.created_at.clone(),
        updated_at: record.updated_at.clone(),
        latest_publication_revision: record
            .latest_publication_revision
            .map(|value| value.to_string()),
        latest_published_article_revision: record
            .latest_published_article_revision
            .map(|value| value.to_string()),
        latest_published_by: record.latest_published_by.clone(),
        latest_published_at: record.latest_published_at.clone(),
    }
}

fn published_response(record: &storage::PublishedRecord) -> PublishArticleResponse {
    PublishArticleResponse {
        article_id: record.article_id.to_string(),
        organization_id: record.organization_id.clone(),
        slug: record.slug.clone(),
        title: record.title.clone(),
        body_markdown: record.body_markdown.clone(),
        article_revision: record.article_revision.to_string(),
        publication_revision: record.publication_revision.to_string(),
        published_by: record.published_by.clone(),
        published_at: record.published_at.clone(),
    }
}

fn get_response(record: &storage::PublishedRecord) -> GetPublishedArticleResponse {
    GetPublishedArticleResponse {
        article_id: record.article_id.to_string(),
        organization_id: record.organization_id.clone(),
        slug: record.slug.clone(),
        title: record.title.clone(),
        body_markdown: record.body_markdown.clone(),
        article_revision: record.article_revision.to_string(),
        publication_revision: record.publication_revision.to_string(),
        published_by: record.published_by.clone(),
        published_at: record.published_at.clone(),
    }
}

fn search_item(record: &storage::PublishedRecord) -> SearchPublishedArticlesResponseArticlesItem {
    SearchPublishedArticlesResponseArticlesItem {
        article_id: record.article_id.to_string(),
        slug: record.slug.clone(),
        title: record.title.clone(),
        article_revision: record.article_revision.to_string(),
        publication_revision: record.publication_revision.to_string(),
        published_at: record.published_at.clone(),
    }
}

fn parse_revision(value: &str) -> Option<i64> {
    value.parse().ok().filter(|value| *value > 0)
}

// The generated portable type preserves missing and explicit null separately.
#[allow(clippy::option_option)]
fn patch_value(value: Option<&Option<String>>) -> Result<Option<&str>, ()> {
    match value {
        None => Ok(None),
        Some(Some(value)) => Ok(Some(value)),
        Some(None) => Err(()),
    }
}

fn valid_callers(values: &[String]) -> bool {
    !values.is_empty()
        && values.len() <= MAX_CALLERS
        && values
            .iter()
            .all(|value| valid_identifier(value, MAX_ID_BYTES))
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

fn valid_identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

fn valid_secret_reference(value: &str) -> bool {
    valid_identifier(value, 256)
        || (!value.is_empty()
            && value.len() <= 256
            && !value.starts_with('/')
            && !value.ends_with('/')
            && !value.contains("//")
            && value.split('/').all(|part| part != "." && part != "..")
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/')
            }))
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_SLUG_BYTES
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_text(value: &str, max: usize, empty_allowed: bool) -> bool {
    value.len() <= max && !value.contains('\0') && (empty_allowed || !value.trim().is_empty())
}

fn valid_idempotency_key(value: &str) -> bool {
    valid_identifier(value, MAX_IDEMPOTENCY_BYTES)
}

fn valid_article_ref(value: &str) -> bool {
    Uuid::parse_str(value).is_ok() || valid_slug(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lenso_auth_sdk::{ActorAssertionIssuer, Validity, audience};
    use lenso_kernel::{CancellationToken, InvocationContext};
    use lenso_native_adapter::NativePluginRegistry;
    use time::Duration as TimeDuration;

    fn config() -> KnowledgeBaseConfig {
        let issuer = ActorAssertionIssuer::new("auth.users", b"knowledge-base-test-key");
        KnowledgeBaseConfig::new(
            "knowledge_base",
            "knowledge-base/database-url",
            "auth.users",
            issuer.public_key_base64(),
            vec!["knowledge-base-api".to_owned()],
        )
        .unwrap()
    }

    fn plugin() -> PostgresKnowledgeBasePlugin {
        PostgresKnowledgeBasePlugin {
            config: config(),
            secrets: Port::default(),
            membership: Port::default(),
            access: Port::default(),
            search: Port::default(),
            search_index: Port::default(),
            prepared: Rc::new(RefCell::new(None)),
        }
    }

    fn context(caller: &str) -> InvocationContext {
        InvocationContext::new(1, None, CancellationToken::new()).with_caller_instance(caller)
    }

    #[test]
    fn descriptor_declares_only_real_capabilities_and_dependencies() {
        let descriptor: serde_json::Value = serde_json::from_str(PLUGIN_DESCRIPTOR_JSON).unwrap();
        let provided = descriptor["provided_capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value["capability_id"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(provided, BTreeSet::from([knowledge::CAPABILITY_ID]));
        let required = descriptor["required_capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value["capability_id"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            required,
            BTreeSet::from([
                secrets::CAPABILITY_ID,
                membership::CAPABILITY_ID,
                access::CAPABILITY_ID,
                search::CAPABILITY_ID,
                search_index::CAPABILITY_ID,
            ])
        );
        assert_eq!(
            NativePluginRegistry::new()
                .with_linked_factories()
                .factories()
                .filter(|factory| factory.package_id() == PACKAGE_ID)
                .count(),
            1
        );
    }

    #[test]
    fn config_and_identifiers_fail_closed() {
        let mut invalid = config();
        invalid.business_callers.clear();
        assert_eq!(
            invalid.validate(),
            Err(KnowledgeBaseConfigError::InvalidBusinessCallers)
        );
        let mut invalid = config();
        invalid
            .business_callers
            .push("knowledge-base-api".to_owned());
        assert_eq!(
            invalid.validate(),
            Err(KnowledgeBaseConfigError::InvalidBusinessCallers)
        );
        assert!(valid_slug("reset-password"));
        assert!(!valid_slug("Reset Password"));
        assert_eq!(parse_revision("1"), Some(1));
        assert_eq!(parse_revision("0"), None);

        let duplicate_grant = PublicReadGrant::new("help-center", "org_acme");
        assert_eq!(
            config().with_public_read_grants(vec![duplicate_grant.clone(), duplicate_grant]),
            Err(KnowledgeBaseConfigError::InvalidPublicReadGrants)
        );
    }

    #[test]
    fn public_read_grants_are_exact_caller_and_organization_pairs() {
        let public_config = config()
            .with_public_read_grants(vec![PublicReadGrant::new("help-center", "org_acme")])
            .unwrap();
        let public_plugin = PostgresKnowledgeBasePlugin {
            config: public_config,
            secrets: Port::default(),
            membership: Port::default(),
            access: Port::default(),
            search: Port::default(),
            search_index: Port::default(),
            prepared: Rc::new(RefCell::new(None)),
        };

        assert!(public_plugin.public_read_allowed(&context("help-center"), "org_acme"));
        assert!(!public_plugin.public_read_allowed(&context("help-center"), "org_other"));
        assert!(!public_plugin.public_read_allowed(&context("other-surface"), "org_acme"));
    }

    #[test]
    fn actor_assertions_are_bound_to_the_exact_operation() {
        let issuer = ActorAssertionIssuer::new("auth.users", b"knowledge-base-test-key");
        let now = OffsetDateTime::now_utc();
        let assertion = issuer.issue(
            "usr_editor",
            "user",
            "strong",
            [audience(
                knowledge::CAPABILITY_ID,
                knowledge::CREATE_DRAFT_OPERATION,
            )],
            Validity::new(
                now - TimeDuration::seconds(1),
                now + TimeDuration::minutes(1),
            )
            .unwrap(),
            std::collections::BTreeMap::default(),
        );
        let attached = assertion.attach(context("knowledge-base-api")).unwrap();
        let verifier = config().verifier().unwrap();
        assert!(
            verifier
                .project_context::<KnowledgeBaseActor>(
                    &attached,
                    knowledge::CAPABILITY_ID,
                    knowledge::CREATE_DRAFT_OPERATION,
                    &UtcClock,
                )
                .is_ok()
        );
        assert!(
            verifier
                .project_context::<KnowledgeBaseActor>(
                    &attached,
                    knowledge::CAPABILITY_ID,
                    knowledge::PUBLISH_ARTICLE_OPERATION,
                    &UtcClock,
                )
                .is_err()
        );
        assert!(
            verifier
                .project_context::<KnowledgeBaseActor>(
                    &attached,
                    knowledge::CAPABILITY_ID,
                    knowledge::GET_DRAFT_OPERATION,
                    &UtcClock,
                )
                .is_err()
        );
        assert!(
            verifier
                .project_context::<KnowledgeBaseActor>(
                    &attached,
                    knowledge::CAPABILITY_ID,
                    knowledge::LIST_ARTICLES_OPERATION,
                    &UtcClock,
                )
                .is_err()
        );
    }

    #[test]
    fn exact_caller_is_rejected_before_dependency_or_storage_access() {
        let result = futures::executor::block_on(plugin().create_draft(
            context("other-api"),
            CreateDraftRequest {
                organization_id: "org_acme".to_owned(),
                slug: "reset-password".to_owned(),
                title: "Reset a password".to_owned(),
                body_markdown: "Use the recovery flow.".to_owned(),
                idempotency_key: "draft-1".to_owned(),
            },
        ));
        assert_eq!(
            result,
            Err(PluginError::Domain(CreateDraftError::Forbidden))
        );

        let result = futures::executor::block_on(plugin().get_draft(
            context("other-api"),
            GetDraftRequest {
                organization_id: "org_acme".to_owned(),
                article_id: Uuid::nil().to_string(),
            },
        ));
        assert_eq!(result, Err(PluginError::Domain(GetDraftError::Forbidden)));
    }

    #[test]
    fn public_read_grant_never_authorizes_draft_body_access() {
        let public_config = config()
            .with_public_read_grants(vec![PublicReadGrant::new("help-center", "org_acme")])
            .unwrap();
        let public_plugin = PostgresKnowledgeBasePlugin {
            config: public_config,
            secrets: Port::default(),
            membership: Port::default(),
            access: Port::default(),
            search: Port::default(),
            search_index: Port::default(),
            prepared: Rc::new(RefCell::new(None)),
        };
        let result = futures::executor::block_on(public_plugin.get_draft(
            context("help-center"),
            GetDraftRequest {
                organization_id: "org_acme".to_owned(),
                article_id: Uuid::nil().to_string(),
            },
        ));
        assert_eq!(result, Err(PluginError::Domain(GetDraftError::Forbidden)));
    }

    #[test]
    fn generated_provider_dispatch_preserves_new_operation_domain_failures() {
        let endpoint = knowledge::KnowledgeBaseEndpoint::new(plugin());
        let get_result = futures::executor::block_on(
            <knowledge::KnowledgeBaseGetDraft as lenso_kernel::RequestCapability>::invoke_native(
                &endpoint,
                knowledge::GET_DRAFT_OPERATION,
                GetDraftRequest {
                    organization_id: "org_acme".to_owned(),
                    article_id: Uuid::nil().to_string(),
                },
                context("other-api"),
            ),
        );
        assert_eq!(get_result, Ok(Err(GetDraftError::Forbidden)));

        let list_result = futures::executor::block_on(
            <knowledge::KnowledgeBaseListArticles as lenso_kernel::RequestCapability>::invoke_native(
                &endpoint,
                knowledge::LIST_ARTICLES_OPERATION,
                ListArticlesRequest {
                    organization_id: "org_acme".to_owned(),
                    limit: 10,
                    cursor: None,
                },
                context("other-api"),
            ),
        );
        assert_eq!(list_result, Ok(Err(ListArticlesError::Forbidden)));
    }

    #[test]
    fn article_cursor_is_stable_and_list_schema_cannot_carry_body_markdown() {
        let summary = storage::ArticleSummaryRecord {
            article_id: Uuid::new_v4(),
            slug: "reset-password".to_owned(),
            title: "Reset a password".to_owned(),
            revision: 3,
            created_by: "usr_editor".to_owned(),
            updated_by: "usr_editor".to_owned(),
            created_at: "2026-08-31T00:00:00Z".to_owned(),
            updated_at: "2026-08-31T01:00:00Z".to_owned(),
            latest_publication_revision: Some(2),
            latest_published_article_revision: Some(2),
            latest_published_by: Some("usr_publisher".to_owned()),
            latest_published_at: Some("2026-08-31T00:30:00Z".to_owned()),
        };
        let encoded = storage::encode_article_cursor(&summary);
        let decoded = storage::decode_article_cursor(&encoded).unwrap();
        assert_eq!(decoded.article_id, summary.article_id);
        assert_eq!(
            decoded.created_at,
            OffsetDateTime::parse(
                &summary.created_at,
                &time::format_description::well_known::Rfc3339
            )
            .unwrap()
        );
        assert!(storage::decode_article_cursor("not-a-cursor").is_none());
        assert!(storage::decode_article_cursor(&"x".repeat(129)).is_none());

        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../lenso-capability-knowledge-base/schemas/list-articles-response.schema.json"
        ))
        .unwrap();
        let properties = schema["$defs"]["ListArticlesResponseArticlesItem"]["properties"]
            .as_object()
            .unwrap();
        assert!(!properties.contains_key("body_markdown"));
    }
}
