-- Memory embedding vectors (migration phase 1). Applied by
-- storage::embeddings::apply_schema. Idempotent.
DEFINE TABLE IF NOT EXISTS memory_embeddings SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS key ON memory_embeddings TYPE string;
DEFINE FIELD IF NOT EXISTS kind ON memory_embeddings TYPE string;
DEFINE FIELD IF NOT EXISTS generation ON memory_embeddings TYPE int;
DEFINE FIELD IF NOT EXISTS embedding ON memory_embeddings TYPE array<float, 384>;
DEFINE INDEX IF NOT EXISTS memory_embedding_diskann ON memory_embeddings
  FIELDS embedding DISKANN DIMENSION 384 DIST COSINE TYPE F32;
