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

`lenso.knowledge-base@1` provides five portable request Operations:

- `create_draft`
- `update_draft`
- `publish_article`
- `get_published_article`
- `search_published_articles`

Draft mutations produce immutable revisions. Publishing records the exact
article revision and never exposes a newer draft accidentally. Generic Search
returns opaque `knowledge-base-article` references; this Plugin filters and
re-reads those references before returning article summaries.

The create-time slug is the stable article address in v1. Draft updates change
title or Markdown body only, so an unpublished edit cannot silently move the
currently published article.

## Authority

Every Operation requires an exact configured caller. Draft and publication
Operations also require an Auth assertion bound to the exact Capability
Operation, active membership in the requested Organization, and an Access
Control decision for the matching permission. Published get/search may instead
be exposed by an exact `(caller_instance, organization_id)` public-read grant;
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
approval workflow, rendered HTML, analytics, events, or bulk reindex Operation.
Public readership is opt-in per exact surface and Organization. Markdown is
stored and returned as authored; rendering and sanitization belong to the
consuming surface.

This is the initial `lenso.knowledge-base@1` Descriptor at version `1.0.0`;
there is no prior accepted Descriptor against which to claim compatibility.
