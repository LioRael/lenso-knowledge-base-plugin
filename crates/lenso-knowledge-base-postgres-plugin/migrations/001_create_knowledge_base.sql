CREATE TABLE knowledge_base_articles (
    organization_id text NOT NULL,
    article_id uuid NOT NULL,
    slug text NOT NULL,
    current_revision bigint NOT NULL CHECK (current_revision > 0),
    latest_publication_revision bigint,
    created_by text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (organization_id, article_id),
    UNIQUE (organization_id, slug),
    CHECK (latest_publication_revision IS NULL OR latest_publication_revision > 0)
);

CREATE TABLE knowledge_base_article_revisions (
    organization_id text NOT NULL,
    article_id uuid NOT NULL,
    revision bigint NOT NULL CHECK (revision > 0),
    slug text NOT NULL,
    title text NOT NULL,
    body_markdown text NOT NULL,
    created_by text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (organization_id, article_id, revision),
    FOREIGN KEY (organization_id, article_id)
        REFERENCES knowledge_base_articles(organization_id, article_id)
        ON DELETE CASCADE
);

CREATE TABLE knowledge_base_publications (
    organization_id text NOT NULL,
    article_id uuid NOT NULL,
    publication_revision bigint NOT NULL CHECK (publication_revision > 0),
    article_revision bigint NOT NULL CHECK (article_revision > 0),
    published_by text NOT NULL,
    published_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
    PRIMARY KEY (organization_id, article_id, publication_revision),
    FOREIGN KEY (organization_id, article_id)
        REFERENCES knowledge_base_articles(organization_id, article_id)
        ON DELETE CASCADE,
    FOREIGN KEY (organization_id, article_id, article_revision)
        REFERENCES knowledge_base_article_revisions(organization_id, article_id, revision)
);

ALTER TABLE knowledge_base_articles
    ADD CONSTRAINT knowledge_base_articles_latest_publication_fk
    FOREIGN KEY (organization_id, article_id, latest_publication_revision)
    REFERENCES knowledge_base_publications(organization_id, article_id, publication_revision)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE knowledge_base_command_receipts (
    caller_instance text NOT NULL,
    operation text NOT NULL,
    idempotency_key text NOT NULL,
    request_hash bytea NOT NULL,
    response_json jsonb,
    index_synchronized boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
    completed_at timestamptz,
    PRIMARY KEY (caller_instance, operation, idempotency_key),
    CHECK ((response_json IS NULL) = (completed_at IS NULL))
);

CREATE INDEX knowledge_base_articles_publication_idx
    ON knowledge_base_articles(organization_id, latest_publication_revision)
    WHERE latest_publication_revision IS NOT NULL;
