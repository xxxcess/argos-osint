# SurrealDB RocksDB endpoint options

This crate vendors the library sources from `surrealdb-kvs-any` 3.3.4 under the included [Business Source License](LICENSE). The workspace uses the local copy through `[patch.crates-io]`. [Pinned upstream source](https://github.com/surrealdb/surrealdb/tree/1ac2b1dba7cd177621a4cf5d507caebeb2414395/surrealdb/kvs-any)

SurrealDB 3.3.4 prefixes datastore URL query keys with `datastore_`. Its RocksDB configuration looks for resource options under `rocksdb_`. The change in [src/registry.rs](src/registry.rs) preserves the seven RocksDB keys used by the Embedded resource profile when the datastore scheme is `rocksdb`. The other query keys retain the upstream mapping. The pinned SDK passes the profile from the Embedded open path. [Upstream RocksDB option parser](https://github.com/surrealdb/surrealdb/blob/1ac2b1dba7cd177621a4cf5d507caebeb2414395/surrealdb/kvs-rocksdb/src/cnf.rs#L656-L722)

On a SurrealDB upgrade, remove this patch when the SDK can pass those options to RocksDB. Verify the production Embedded open path with `embedded_open_applies_rocksdb_resource_profile`.
