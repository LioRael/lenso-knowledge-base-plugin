//! Agent-facing Tools over an explicitly bound Knowledge Base capability.

use lenso::prelude::*;
use lenso_capability_agent_tool_provider::{
    self as tool_contract, CatalogRequest, CatalogResponse, ContentType, ExecuteError,
    ExecuteRequest, ExecuteResponse, ExecutionFailedPayload, ToolDefinition, ToolExecutionClass,
};
use lenso_capability_knowledge_base::{
    self as knowledge_base, CreateDraftRequest, GetDraftRequest, GetPublishedArticleRequest,
    ListArticlesRequest, PublishArticleRequest, SearchPublishedArticlesRequest, UpdateDraftRequest,
};
use lenso_kernel::RuntimeFailure;
use serde::{Serialize, de::DeserializeOwned};

pub const CREATE_DRAFT_TOOL: &str = "knowledge_base_create_draft";
pub const UPDATE_DRAFT_TOOL: &str = "knowledge_base_update_draft";
pub const GET_DRAFT_TOOL: &str = "knowledge_base_get_draft";
pub const LIST_ARTICLES_TOOL: &str = "knowledge_base_list_articles";
pub const PUBLISH_ARTICLE_TOOL: &str = "knowledge_base_publish_article";
pub const GET_PUBLISHED_ARTICLE_TOOL: &str = "knowledge_base_get_published_article";
pub const SEARCH_PUBLISHED_ARTICLES_TOOL: &str = "knowledge_base_search_published_articles";

#[lenso::plugin]
#[derive(Clone, Debug)]
struct KnowledgeBaseAgentToolsPlugin {
    knowledge_base: Port<knowledge_base::KnowledgeBaseClient>,
}

#[lenso::provides(tool_contract::ToolProvider)]
impl KnowledgeBaseAgentToolsPlugin {
    fn catalog(
        &self,
        _context: Ctx,
        _request: CatalogRequest,
    ) -> impl std::future::Future<Output = PluginResult<CatalogResponse, tool_contract::CatalogError>>
    {
        let _ = self;
        futures::future::ready(Ok(CatalogResponse {
            tools: tool_definitions(),
        }))
    }

    async fn execute(
        &self,
        context: Ctx,
        request: ExecuteRequest,
    ) -> PluginResult<ExecuteResponse, ExecuteError> {
        macro_rules! invoke {
            ($future:expr, $tool:expr, $domain:path, $runtime:path) => {
                match $future.await {
                    Ok(response) => success($tool, &response),
                    Err($domain(error)) => Err(PluginError::domain(map_domain_error(&error))),
                    Err($runtime(error)) => Err(PluginError::runtime(error)),
                }
            };
        }

        match request.name.as_str() {
            CREATE_DRAFT_TOOL => {
                let arguments = decode::<CreateDraftRequest>(&request)?;
                invoke!(
                    self.knowledge_base
                        .create_draft_with_context(context, arguments),
                    CREATE_DRAFT_TOOL,
                    knowledge_base::KnowledgeBaseCreateDraftInvocationError::Domain,
                    knowledge_base::KnowledgeBaseCreateDraftInvocationError::Runtime
                )
            }
            UPDATE_DRAFT_TOOL => {
                let arguments = decode::<UpdateDraftRequest>(&request)?;
                invoke!(
                    self.knowledge_base
                        .update_draft_with_context(context, arguments),
                    UPDATE_DRAFT_TOOL,
                    knowledge_base::KnowledgeBaseUpdateDraftInvocationError::Domain,
                    knowledge_base::KnowledgeBaseUpdateDraftInvocationError::Runtime
                )
            }
            GET_DRAFT_TOOL => {
                let arguments = decode::<GetDraftRequest>(&request)?;
                invoke!(
                    self.knowledge_base
                        .get_draft_with_context(context, arguments),
                    GET_DRAFT_TOOL,
                    knowledge_base::KnowledgeBaseGetDraftInvocationError::Domain,
                    knowledge_base::KnowledgeBaseGetDraftInvocationError::Runtime
                )
            }
            LIST_ARTICLES_TOOL => {
                let arguments = decode::<ListArticlesRequest>(&request)?;
                invoke!(
                    self.knowledge_base
                        .list_articles_with_context(context, arguments),
                    LIST_ARTICLES_TOOL,
                    knowledge_base::KnowledgeBaseListArticlesInvocationError::Domain,
                    knowledge_base::KnowledgeBaseListArticlesInvocationError::Runtime
                )
            }
            PUBLISH_ARTICLE_TOOL => {
                let arguments = decode::<PublishArticleRequest>(&request)?;
                invoke!(
                    self.knowledge_base
                        .publish_article_with_context(context, arguments),
                    PUBLISH_ARTICLE_TOOL,
                    knowledge_base::KnowledgeBasePublishArticleInvocationError::Domain,
                    knowledge_base::KnowledgeBasePublishArticleInvocationError::Runtime
                )
            }
            GET_PUBLISHED_ARTICLE_TOOL => {
                let arguments = decode::<GetPublishedArticleRequest>(&request)?;
                invoke!(
                    self.knowledge_base
                        .get_published_article_with_context(context, arguments),
                    GET_PUBLISHED_ARTICLE_TOOL,
                    knowledge_base::KnowledgeBaseGetPublishedArticleInvocationError::Domain,
                    knowledge_base::KnowledgeBaseGetPublishedArticleInvocationError::Runtime
                )
            }
            SEARCH_PUBLISHED_ARTICLES_TOOL => {
                let arguments = decode::<SearchPublishedArticlesRequest>(&request)?;
                invoke!(
                    self.knowledge_base
                        .search_published_articles_with_context(context, arguments),
                    SEARCH_PUBLISHED_ARTICLES_TOOL,
                    knowledge_base::KnowledgeBaseSearchPublishedArticlesInvocationError::Domain,
                    knowledge_base::KnowledgeBaseSearchPublishedArticlesInvocationError::Runtime
                )
            }
            _ => Err(PluginError::domain(ExecuteError::NotFound)),
        }
    }
}

fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        tool(
            GET_DRAFT_TOOL,
            "Get the latest draft revision and Markdown body for one article. Requires author edit authority.",
            include_str!(
                "../../lenso-capability-knowledge-base/schemas/get-draft-request.schema.json"
            ),
            ToolExecutionClass::ParallelSafe,
        ),
        tool(
            LIST_ARTICLES_TOOL,
            "List author-visible article summaries with stable bounded pagination; Markdown bodies are omitted.",
            include_str!(
                "../../lenso-capability-knowledge-base/schemas/list-articles-request.schema.json"
            ),
            ToolExecutionClass::ParallelSafe,
        ),
        tool(
            GET_PUBLISHED_ARTICLE_TOOL,
            "Get the latest published revision of one article by stable article ID or slug.",
            include_str!(
                "../../lenso-capability-knowledge-base/schemas/get-published-article-request.schema.json"
            ),
            ToolExecutionClass::ParallelSafe,
        ),
        tool(
            SEARCH_PUBLISHED_ARTICLES_TOOL,
            "Search published articles and return only results re-read and authorized by Knowledge Base.",
            include_str!(
                "../../lenso-capability-knowledge-base/schemas/search-published-articles-request.schema.json"
            ),
            ToolExecutionClass::ParallelSafe,
        ),
        tool(
            CREATE_DRAFT_TOOL,
            "Create one article draft with a stable slug. Reuse the same idempotency_key when retrying the same intent.",
            include_str!(
                "../../lenso-capability-knowledge-base/schemas/create-draft-request.schema.json"
            ),
            ToolExecutionClass::Exclusive,
        ),
        tool(
            UPDATE_DRAFT_TOOL,
            "Create a new immutable draft revision using the latest revision from get_draft. Reuse the same idempotency_key for retries.",
            include_str!(
                "../../lenso-capability-knowledge-base/schemas/update-draft-request.schema.json"
            ),
            ToolExecutionClass::Exclusive,
        ),
        tool(
            PUBLISH_ARTICLE_TOOL,
            "Publish one exact article revision. Reuse the same idempotency_key to resume safe Search synchronization after a runtime failure.",
            include_str!(
                "../../lenso-capability-knowledge-base/schemas/publish-article-request.schema.json"
            ),
            ToolExecutionClass::Exclusive,
        ),
    ]
}

fn tool(
    name: &str,
    description: &str,
    schema: &str,
    execution: ToolExecutionClass,
) -> ToolDefinition {
    let schema: serde_json::Value =
        serde_json::from_str(schema).expect("Knowledge Base Tool schema must be valid JSON");
    ToolDefinition {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema_json: schema
            .to_string()
            .try_into()
            .expect("Knowledge Base Tool schema must remain valid JSON"),
        execution,
    }
}

fn decode<T: DeserializeOwned>(request: &ExecuteRequest) -> PluginResult<T, ExecuteError> {
    serde_json::from_str(request.arguments_json.as_str())
        .map_err(|_| PluginError::domain(ExecuteError::InvalidArguments))
}

fn success<T: Serialize>(
    tool_name: &str,
    response: &T,
) -> PluginResult<ExecuteResponse, ExecuteError> {
    let content = serde_json::to_string_pretty(response).map_err(|error| {
        PluginError::runtime(RuntimeFailure::PluginFailure {
            detail: format!("Knowledge Base Tool could not serialize its typed response: {error}"),
        })
    })?;
    Ok(ExecuteResponse {
        content_blocks: None,
        content,
        content_type: ContentType::Text,
        metadata_json: serde_json::json!({ "tool": tool_name })
            .to_string()
            .try_into()
            .expect("Knowledge Base Tool metadata must be valid JSON"),
    })
}

trait DomainToolError {
    fn to_tool_error(&self) -> ExecuteError;
}

fn map_domain_error(error: &impl DomainToolError) -> ExecuteError {
    error.to_tool_error()
}

fn rejected(reason_code: &str) -> ExecuteError {
    ExecuteError::ExecutionFailed {
        payload: ExecutionFailedPayload {
            reason_code: reason_code.to_owned(),
            message: "Knowledge Base rejected the requested operation.".to_owned(),
            details_json: serde_json::json!({ "domain_error": reason_code })
                .to_string()
                .try_into()
                .expect("Knowledge Base Tool error metadata must be valid JSON"),
        },
    }
}

macro_rules! impl_article_read_error {
    ($($error:ty),+ $(,)?) => {
        $(
            impl DomainToolError for $error {
                fn to_tool_error(&self) -> ExecuteError {
                    match self {
                        Self::InvalidRequest => ExecuteError::InvalidArguments,
                        Self::ArticleNotFound => ExecuteError::NotFound,
                        Self::Forbidden | Self::Unauthenticated => ExecuteError::PermissionDenied,
                        Self::Unknown(_) => rejected("unknown_domain_error"),
                    }
                }
            }
        )+
    };
}

impl_article_read_error!(
    knowledge_base::GetDraftError,
    knowledge_base::GetPublishedArticleError,
);

impl DomainToolError for knowledge_base::ListArticlesError {
    fn to_tool_error(&self) -> ExecuteError {
        match self {
            Self::InvalidRequest => ExecuteError::InvalidArguments,
            Self::Forbidden | Self::Unauthenticated => ExecuteError::PermissionDenied,
            Self::Unknown(_) => rejected("unknown_domain_error"),
        }
    }
}

impl DomainToolError for knowledge_base::SearchPublishedArticlesError {
    fn to_tool_error(&self) -> ExecuteError {
        match self {
            Self::InvalidQuery => ExecuteError::InvalidArguments,
            Self::Forbidden | Self::Unauthenticated => ExecuteError::PermissionDenied,
            Self::Unknown(_) => rejected("unknown_domain_error"),
        }
    }
}

impl DomainToolError for knowledge_base::CreateDraftError {
    fn to_tool_error(&self) -> ExecuteError {
        match self {
            Self::InvalidRequest => ExecuteError::InvalidArguments,
            Self::Forbidden | Self::Unauthenticated => ExecuteError::PermissionDenied,
            Self::IdempotencyConflict => rejected("idempotency_conflict"),
            Self::SlugConflict => rejected("slug_conflict"),
            Self::Unknown(_) => rejected("unknown_domain_error"),
        }
    }
}

impl DomainToolError for knowledge_base::UpdateDraftError {
    fn to_tool_error(&self) -> ExecuteError {
        match self {
            Self::InvalidRequest => ExecuteError::InvalidArguments,
            Self::ArticleNotFound => ExecuteError::NotFound,
            Self::Forbidden | Self::Unauthenticated => ExecuteError::PermissionDenied,
            Self::IdempotencyConflict => rejected("idempotency_conflict"),
            Self::RevisionConflict => rejected("revision_conflict"),
            Self::SlugConflict => rejected("slug_conflict"),
            Self::Unknown(_) => rejected("unknown_domain_error"),
        }
    }
}

impl DomainToolError for knowledge_base::PublishArticleError {
    fn to_tool_error(&self) -> ExecuteError {
        match self {
            Self::InvalidRequest => ExecuteError::InvalidArguments,
            Self::ArticleNotFound => ExecuteError::NotFound,
            Self::Forbidden | Self::Unauthenticated => ExecuteError::PermissionDenied,
            Self::IdempotencyConflict => rejected("idempotency_conflict"),
            Self::RevisionConflict => rejected("revision_conflict"),
            Self::Unknown(_) => rejected("unknown_domain_error"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(name: &str, arguments: &str) -> ExecuteRequest {
        ExecuteRequest {
            name: name.to_owned(),
            arguments_json: arguments.try_into().unwrap(),
        }
    }

    #[test]
    fn descriptor_is_a_removable_adapter_with_one_business_requirement() {
        let descriptor: serde_json::Value = serde_json::from_str(PLUGIN_DESCRIPTOR_JSON).unwrap();
        assert_eq!(descriptor["plugin_id"], "lenso.knowledge-base.agent-tools");
        let provided = descriptor["provided_capabilities"].as_array().unwrap();
        assert_eq!(provided.len(), 1);
        assert_eq!(provided[0]["capability_id"], "lenso.agent.tool-provider@2");
        let required = descriptor["required_capabilities"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0]["capability_id"], "lenso.knowledge-base@1");
    }

    #[test]
    fn catalog_has_four_parallel_reads_and_three_exclusive_mutations() {
        let tools = tool_definitions();
        assert_eq!(tools.len(), 7);
        assert_eq!(
            tools
                .iter()
                .filter(|tool| tool.execution == ToolExecutionClass::ParallelSafe)
                .count(),
            4
        );
        assert_eq!(
            tools
                .iter()
                .filter(|tool| tool.execution == ToolExecutionClass::Exclusive)
                .count(),
            3
        );
        assert!(tools.iter().all(|tool| {
            let schema: serde_json::Value =
                serde_json::from_str(tool.input_schema_json.as_str()).unwrap();
            schema["additionalProperties"] == false
        }));
    }

    #[test]
    fn exact_capability_requests_decode_without_adapter_owned_business_fields() {
        let get = decode::<GetPublishedArticleRequest>(&request(
            GET_PUBLISHED_ARTICLE_TOOL,
            r#"{"organization_id":"org-1","article_ref":"getting-started"}"#,
        ))
        .unwrap();
        assert_eq!(get.article_ref, "getting-started");

        assert!(
            decode::<GetPublishedArticleRequest>(&request(
                GET_PUBLISHED_ARTICLE_TOOL,
                r#"{"article_ref":42}"#,
            ))
            .is_err()
        );
    }

    #[test]
    fn authorization_not_found_and_revision_failures_remain_distinct() {
        assert_eq!(
            map_domain_error(&knowledge_base::GetDraftError::Forbidden),
            ExecuteError::PermissionDenied
        );
        assert_eq!(
            map_domain_error(&knowledge_base::GetDraftError::ArticleNotFound),
            ExecuteError::NotFound
        );
        let ExecuteError::ExecutionFailed { payload } =
            map_domain_error(&knowledge_base::PublishArticleError::RevisionConflict)
        else {
            panic!("revision conflict must remain an execution failure");
        };
        assert_eq!(payload.reason_code, "revision_conflict");
    }
}
