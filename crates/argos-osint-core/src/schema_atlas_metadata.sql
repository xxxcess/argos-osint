CREATE TABLE IF NOT EXISTS atlas_article_metadata (
 run_id TEXT NOT NULL, article_id TEXT NOT NULL, input_hash TEXT NOT NULL,
 entity_state TEXT NOT NULL, reliability_state TEXT NOT NULL,
 classification_status TEXT NOT NULL, metadata_json TEXT NOT NULL,
 PRIMARY KEY(run_id,article_id),
 FOREIGN KEY(run_id,article_id) REFERENCES atlas_articles(run_id,article_id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS atlas_article_mentions (
 run_id TEXT NOT NULL, article_id TEXT NOT NULL, type TEXT NOT NULL,
 comparison_key TEXT NOT NULL, field TEXT NOT NULL, start_byte INTEGER NOT NULL,
 end_byte INTEGER NOT NULL, original_text TEXT NOT NULL, input_hash TEXT NOT NULL,
 PRIMARY KEY(run_id,article_id,type,comparison_key,field,start_byte,end_byte),
 FOREIGN KEY(run_id,article_id) REFERENCES atlas_articles(run_id,article_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS atlas_mentions_key ON atlas_article_mentions(run_id,type,comparison_key);
CREATE TABLE IF NOT EXISTS atlas_related_articles (
 run_id TEXT NOT NULL, left_id TEXT NOT NULL, right_id TEXT NOT NULL,
 left_revision TEXT NOT NULL, right_revision TEXT NOT NULL,
 shared_json TEXT NOT NULL, match_version TEXT NOT NULL,
 named_count INTEGER NOT NULL, shared_count INTEGER NOT NULL, rarity REAL NOT NULL,
 PRIMARY KEY(run_id,left_id,right_id), CHECK(left_id < right_id),
 FOREIGN KEY(run_id,left_id) REFERENCES atlas_articles(run_id,article_id) ON DELETE CASCADE,
 FOREIGN KEY(run_id,right_id) REFERENCES atlas_articles(run_id,article_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS atlas_related_left ON atlas_related_articles(run_id,left_id);
CREATE INDEX IF NOT EXISTS atlas_related_right ON atlas_related_articles(run_id,right_id);
CREATE TRIGGER IF NOT EXISTS atlas_metadata_invalidate_source AFTER UPDATE ON atlas_articles
WHEN old.title <> new.title OR old.description <> new.description OR old.url <> new.url
 OR old.source_domain <> new.source_domain OR old.source_name <> new.source_name
BEGIN
 DELETE FROM atlas_article_metadata WHERE run_id=new.run_id AND article_id=new.article_id;
 DELETE FROM atlas_article_mentions WHERE run_id=new.run_id AND article_id=new.article_id;
 DELETE FROM atlas_related_articles WHERE run_id=new.run_id AND (left_id=new.article_id OR right_id=new.article_id);
END;
CREATE TRIGGER IF NOT EXISTS atlas_related_invalidate_category AFTER UPDATE OF category ON atlas_articles
WHEN old.category <> new.category
BEGIN
 DELETE FROM atlas_related_articles WHERE run_id=new.run_id AND (left_id=new.article_id OR right_id=new.article_id);
END;
