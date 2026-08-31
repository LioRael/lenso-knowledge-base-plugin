CREATE INDEX knowledge_base_articles_author_list_idx
    ON knowledge_base_articles(organization_id, created_at DESC, article_id DESC);
