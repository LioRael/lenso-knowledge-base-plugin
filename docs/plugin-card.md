# Knowledge Base v1 Plugin card

## Owner and deletion boundary

`lenso-knowledge-base-postgres-plugin` owns articles, immutable article
revisions, publication history, latest-publication pointers, command replay
receipts, and index-synchronization receipts. Removing its package, Instance,
Capability bindings, and owned PostgreSQL schema removes Knowledge Base
behavior without deleting Organizations, membership, Auth identities, Access
Control policy, Secrets, or Search itself. Search documents are rebuildable and
may be removed independently.

## Capability and observable behavior

`lenso.knowledge-base@1` provides seven portable request Operations:

- `create_draft`
- `update_draft`
- `get_draft`
- `list_articles`
- `publish_article`
- `get_published_article`
- `search_published_articles`

The separate `lenso.knowledge-base.agent-tools` adapter provides
`lenso.agent.tool-provider@2` and requires exactly one
`lenso.knowledge-base@1` provider. It owns only the Agent catalog and typed
argument/result adaptation. Removing it removes the Agent surface without
removing Knowledge Base facts, Search documents, or publication history.

Draft mutations produce immutable revisions. Publishing records the exact
article revision and never exposes a newer draft accidentally. Generic Search
returns opaque `knowledge-base-article` references; this Plugin filters and
re-reads those references before returning article summaries.

`list_articles` is bounded to 100 entries and pages newest-created articles by
the immutable `(created_at, article_id)` keyset. Editing or publishing an
article therefore does not move it across an in-progress pagination walk. Its
summary includes the latest draft revision and enough publication metadata to
distinguish an unpublished change, but its query and response shape omit the
Markdown body. `get_draft` is the only author read that returns that body.

The create-time slug is the stable article address in v1. Draft updates change
title or Markdown body only, so an unpublished edit cannot silently move the
currently published article.

## Authority

Every Operation requires an exact configured caller. Draft reads, draft
mutations, and publication Operations also require an Auth assertion bound to
the exact Capability Operation, active membership in the requested
Organization, and an Access Control decision for the matching permission.
`get_draft` and `list_articles` deliberately reuse the existing
`knowledge-base.articles.edit` permission: a public-read grant is never enough
to obtain unpublished metadata or a body. Published get/search may instead be
exposed by an exact `(caller_instance, organization_id)` public-read grant;
that narrow grant cannot read drafts or cross into another Organization.
Knowledge Base makes the final decision and source re-read. Search ranking is
never treated as authorization.

Permissions are:

- `knowledge-base.articles.create`
- `knowledge-base.articles.edit`
- `knowledge-base.articles.publish`
- `knowledge-base.articles.read`
- `knowledge-base.articles.search`

## State and lifecycle

Setup and upgrade are explicit operator workflows. Activation resolves the
database URL through `lenso.secrets@1` and verifies the authored schema. All
article, revision, publication, replay, and Search-sync facts survive restart.
Each prepared App Generation owns a fresh PostgreSQL handle and closes it on
deactivation.

Publishing commits canonical state before calling `lenso.search-index@1`.
Search indexing is deliberately not part of the source transaction. If the
dependency is unavailable, the request reports a Runtime Failure; replaying the
same idempotency key resumes synchronization without creating another
publication. Synchronization re-reads the current latest publication while
holding the same article-scoped PostgreSQL advisory lock used by publishing, so
an older receipt cannot race a newer publication and restore stale Search text.

## Privacy contract fit

This release does not provide `lenso.data-export-source@1` or
`lenso.retention-participant@1`. Organization-owned editorial content is not a
subject-owned dataset, and the current privacy contracts are subject-scoped.
Adding them would invent misleading deletion semantics for shared articles.

## Honest limits

v1 has no locale variants, attachments, article deletion/unpublish, editorial
approval workflow, rendered HTML, authoring Web surface, analytics, events, or
bulk reindex Operation. Public readership is opt-in per exact surface and
Organization. Markdown is stored and returned as authored; rendering and
sanitization belong to the consuming surface.

Descriptor `1.1.0` adds `get_draft` and `list_articles` without changing the
existing five Operations. `lenso-contract-codegen lint` accepts the change
against the published `1.0.0` Descriptor as an additive compatible minor
evolution.
