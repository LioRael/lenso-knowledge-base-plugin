# Lenso Knowledge Base Plugin

This repository owns one removable Knowledge Base behavior boundary for Lenso:
draft articles, immutable revisions, explicit publications, and durable command
idempotency in PostgreSQL.

`lenso.knowledge-base@1` is a portable request Capability. It lets trusted
product surfaces create and update drafts, publish one exact revision, re-read
the latest published article, and search published article references. Every
request requires an exact caller. Mutations require an operation-bound Auth
assertion, active Organization membership, and Access Control approval;
published reads may instead use an explicit caller-and-Organization public-read
grant. Search returns only articles that this Plugin re-read from its
source-of-truth tables after that authority decision.

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
and enable `postgres-acceptance` to run restart/idempotency/revision tests.
Schema setup and upgrade are explicit operator actions; activation only verifies
and opens an already prepared schema.
