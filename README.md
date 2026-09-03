# Lenso Knowledge Base Plugin

This repository owns one removable Knowledge Base behavior boundary for Lenso:
draft articles, immutable revisions, explicit publications, and durable command
idempotency in PostgreSQL.

`lenso.knowledge-base@1` is a portable request Capability. It lets trusted
product surfaces create and update drafts, fetch one current draft, page
author-safe article summaries, publish one exact revision, re-read the latest
published article, and search published article references. The author list is
bounded and uses a stable `(created_at, article_id)` keyset cursor; it carries
revision, author, update, and publication metadata but never Markdown bodies.
Every request requires an exact caller. Draft reads and mutations require an
operation-bound Auth assertion, active Organization membership, and Access
Control approval; published reads may instead use an explicit
caller-and-Organization public-read grant. Search returns only articles that
this Plugin re-read from its source-of-truth tables after that authority
decision.

The separate `lenso.knowledge-base.agent-tools` linked Plugin exposes the same
seven bounded operations to App Agents. It forwards invocation context
unchanged, so Knowledge Base remains the final owner of exact-caller admission,
public-read grants, authentication, membership, authorization, revision checks,
idempotency, publication state, and Search source re-reading.

The Plugin requires `lenso.secrets@1`, `lenso.organization-membership@1`,
`lenso.access-control@1`, `lenso.search@1`, and `lenso.search-index@1`. Search
owns only a rebuildable index. Publishing commits the canonical publication
first, then upserts the Search document; a Runtime Failure leaves a durable,
safe-to-retry publication receipt. The same idempotency key re-reads and
synchronizes the current latest publication under an article-scoped PostgreSQL
advisory lock, so replaying an older receipt cannot overwrite a newer index
document.

## Repository layout

- `crates/lenso-capability-knowledge-base`: authoritative portable contract,
  Schemas, generated Rust Provider/Client, and freshness gate.
- `crates/lenso-knowledge-base-agent-tools-plugin`: stateless, removable Agent
  Tool catalog and typed argument/result adapter.
- `crates/lenso-knowledge-base-postgres-plugin`: linked native provider,
  authorization, PostgreSQL state, Search collaboration, and operator schema
  workflows.
- `docs/plugin-card.md`: ownership, deletion, lifecycle, and honest limits.

## Local verification

```sh
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
./scripts/check-repository-boundary.sh
```

Set `LENSO_KNOWLEDGE_BASE_TEST_DATABASE_URL` to a dedicated PostgreSQL database
and enable `postgres-acceptance` to run restart, idempotency, revision, draft
read, Organization isolation, and stable-pagination tests. Schema setup and
upgrade are explicit operator actions; activation only verifies and opens an
already prepared schema.
