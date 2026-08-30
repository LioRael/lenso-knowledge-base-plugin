//! Authoritative source for `lenso.knowledge-base@1`.

use lenso_contract_authoring as lenso;

#[derive(serde::Deserialize)]
pub struct Nullable<T>(Option<T>);

impl<T: lenso::JsonSchema> lenso::JsonSchema for Nullable<T> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!("Nullable_{}", T::schema_name()).into()
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        format!("Nullable<{}>", T::schema_id()).into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        <Option<T> as lenso::JsonSchema>::json_schema(generator)
    }
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct CreateDraftRequest {
    pub organization_id: String,
    pub slug: String,
    pub title: String,
    pub body_markdown: String,
    pub idempotency_key: String,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct CreateDraftResponse {
    pub article_id: String,
    pub organization_id: String,
    pub slug: String,
    pub title: String,
    pub body_markdown: String,
    pub revision: String,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(lenso::DomainError)]
pub enum CreateDraftError {
    InvalidRequest,
    Unauthenticated,
    Forbidden,
    SlugConflict,
    IdempotencyConflict,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct UpdateDraftRequest {
    pub organization_id: String,
    pub article_id: String,
    pub expected_revision: String,
    pub title: Option<String>,
    pub body_markdown: Option<String>,
    pub idempotency_key: String,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct UpdateDraftResponse {
    pub article_id: String,
    pub organization_id: String,
    pub slug: String,
    pub title: String,
    pub body_markdown: String,
    pub revision: String,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(lenso::DomainError)]
pub enum UpdateDraftError {
    InvalidRequest,
    Unauthenticated,
    Forbidden,
    ArticleNotFound,
    RevisionConflict,
    SlugConflict,
    IdempotencyConflict,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GetDraftRequest {
    pub organization_id: String,
    pub article_id: String,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GetDraftResponse {
    pub article_id: String,
    pub organization_id: String,
    pub slug: String,
    pub title: String,
    pub body_markdown: String,
    pub revision: String,
    pub created_by: String,
    pub updated_by: String,
    #[schemars(extend("format" = "date-time"))]
    pub created_at: String,
    #[schemars(extend("format" = "date-time"))]
    pub updated_at: String,
    pub latest_publication_revision: Nullable<String>,
    pub latest_published_article_revision: Nullable<String>,
    pub latest_published_by: Nullable<String>,
    #[schemars(extend("format" = "date-time"))]
    pub latest_published_at: Nullable<String>,
}

#[derive(lenso::DomainError)]
pub enum GetDraftError {
    InvalidRequest,
    Unauthenticated,
    Forbidden,
    ArticleNotFound,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct ListArticlesRequest {
    pub organization_id: String,
    #[schemars(range(min = 1, max = 100))]
    pub limit: i64,
    pub cursor: Nullable<String>,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct ListArticlesResponseArticlesItem {
    pub article_id: String,
    pub slug: String,
    pub title: String,
    pub revision: String,
    pub created_by: String,
    pub updated_by: String,
    #[schemars(extend("format" = "date-time"))]
    pub created_at: String,
    #[schemars(extend("format" = "date-time"))]
    pub updated_at: String,
    pub latest_publication_revision: Nullable<String>,
    pub latest_published_article_revision: Nullable<String>,
    pub latest_published_by: Nullable<String>,
    #[schemars(extend("format" = "date-time"))]
    pub latest_published_at: Nullable<String>,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct ListArticlesResponse {
    pub articles: Vec<ListArticlesResponseArticlesItem>,
    pub next_cursor: Nullable<String>,
}

#[derive(lenso::DomainError)]
pub enum ListArticlesError {
    InvalidRequest,
    Unauthenticated,
    Forbidden,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct PublishArticleRequest {
    pub organization_id: String,
    pub article_id: String,
    pub expected_revision: String,
    pub idempotency_key: String,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct PublishArticleResponse {
    pub article_id: String,
    pub organization_id: String,
    pub slug: String,
    pub title: String,
    pub body_markdown: String,
    pub article_revision: String,
    pub publication_revision: String,
    pub published_by: String,
    pub published_at: String,
}

#[derive(lenso::DomainError)]
pub enum PublishArticleError {
    InvalidRequest,
    Unauthenticated,
    Forbidden,
    ArticleNotFound,
    RevisionConflict,
    IdempotencyConflict,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GetPublishedArticleRequest {
    pub organization_id: String,
    pub article_ref: String,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct GetPublishedArticleResponse {
    pub article_id: String,
    pub organization_id: String,
    pub slug: String,
    pub title: String,
    pub body_markdown: String,
    pub article_revision: String,
    pub publication_revision: String,
    pub published_by: String,
    pub published_at: String,
}

#[derive(lenso::DomainError)]
pub enum GetPublishedArticleError {
    InvalidRequest,
    Unauthenticated,
    Forbidden,
    ArticleNotFound,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct SearchPublishedArticlesRequest {
    pub organization_id: String,
    pub query: String,
    #[schemars(range(min = 1, max = 100))]
    pub limit: i64,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct SearchPublishedArticlesResponseArticlesItem {
    pub article_id: String,
    pub slug: String,
    pub title: String,
    pub article_revision: String,
    pub publication_revision: String,
    pub published_at: String,
}

#[derive(lenso::JsonSchema, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[schemars(deny_unknown_fields)]
pub struct SearchPublishedArticlesResponse {
    pub articles: Vec<SearchPublishedArticlesResponseArticlesItem>,
    pub index_revision: String,
}

#[derive(lenso::DomainError)]
pub enum SearchPublishedArticlesError {
    InvalidQuery,
    Unauthenticated,
    Forbidden,
}

#[lenso::capability(
    id = "lenso.knowledge-base",
    major = 1,
    version = "1.1.0",
    portable = true,
    cross_lane_transfer = true
)]
pub trait KnowledgeBase {
    async fn create_draft(
        &self,
        context: lenso::Ctx<'_>,
        request: CreateDraftRequest,
    ) -> Result<CreateDraftResponse, CreateDraftError>;

    async fn update_draft(
        &self,
        context: lenso::Ctx<'_>,
        request: UpdateDraftRequest,
    ) -> Result<UpdateDraftResponse, UpdateDraftError>;

    async fn get_draft(
        &self,
        context: lenso::Ctx<'_>,
        request: GetDraftRequest,
    ) -> Result<GetDraftResponse, GetDraftError>;

    async fn list_articles(
        &self,
        context: lenso::Ctx<'_>,
        request: ListArticlesRequest,
    ) -> Result<ListArticlesResponse, ListArticlesError>;

    async fn publish_article(
        &self,
        context: lenso::Ctx<'_>,
        request: PublishArticleRequest,
    ) -> Result<PublishArticleResponse, PublishArticleError>;

    async fn get_published_article(
        &self,
        context: lenso::Ctx<'_>,
        request: GetPublishedArticleRequest,
    ) -> Result<GetPublishedArticleResponse, GetPublishedArticleError>;

    async fn search_published_articles(
        &self,
        context: lenso::Ctx<'_>,
        request: SearchPublishedArticlesRequest,
    ) -> Result<SearchPublishedArticlesResponse, SearchPublishedArticlesError>;
}
