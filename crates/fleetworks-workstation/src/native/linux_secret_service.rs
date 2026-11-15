//! Linux D-Bus Secret Service adapter (`org.freedesktop.secrets`).
//!
//! Call only on a blocking worker, never on a UI callback or async executor thread.
//! Does not fall back to the XDG file-based Portal backend, CLI helpers, env vars,
//! or plaintext JSON storage.

#[cfg(any(target_os = "linux", test))]
use std::collections::BTreeMap;

#[cfg(any(target_os = "linux", test))]
use super::keychain::KeychainError;
#[cfg(any(target_os = "linux", test))]
use super::keychain::KeychainItem;
#[cfg(target_os = "linux")]
use super::keychain::KeychainStore;
#[cfg(any(target_os = "linux", test))]
use super::keychain::SecretToken;

#[cfg(any(target_os = "linux", test))]
const APPLICATION_ATTRIBUTE: &str = "dev.fleetworks.native";
#[cfg(any(target_os = "linux", test))]
const ATTR_APPLICATION: &str = "application";
#[cfg(any(target_os = "linux", test))]
const ATTR_SERVICE: &str = "service";
#[cfg(any(target_os = "linux", test))]
const ATTR_ACCOUNT: &str = "account";

#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SecretRecord {
    pub(crate) attributes: BTreeMap<String, String>,
    pub(crate) locked: bool,
    pub(crate) secret: Vec<u8>,
}

/// Abstract Secret Service collection backend so matching and error handling run in unit tests.
#[cfg(any(target_os = "linux", test))]
pub(crate) trait SecretServiceBackend {
    fn search_items(
        &self,
        attributes: &BTreeMap<String, String>,
    ) -> Result<Vec<SecretRecord>, KeychainError>;

    fn write_item(
        &self,
        label: &str,
        attributes: &BTreeMap<String, String>,
        secret: &[u8],
    ) -> Result<(), KeychainError>;

    fn delete_matching(
        &self,
        attributes: &BTreeMap<String, String>,
    ) -> Result<usize, KeychainError>;
}

/// Linux system keyring backed exclusively by D-Bus Secret Service.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LinuxSecretService;

#[cfg(target_os = "linux")]
impl KeychainStore for LinuxSecretService {
    fn get(&self, item: &KeychainItem) -> Result<SecretToken, KeychainError> {
        read_with_backend(&live::DbusSecretBackend, item)
    }

    fn set(&self, item: &KeychainItem, token: &SecretToken) -> Result<(), KeychainError> {
        write_with_backend(&live::DbusSecretBackend, item, token)
    }

    fn delete(&self, item: &KeychainItem) -> Result<(), KeychainError> {
        delete_with_backend(&live::DbusSecretBackend, item)
    }
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn item_attributes(item: &KeychainItem) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            ATTR_APPLICATION.to_owned(),
            APPLICATION_ATTRIBUTE.to_owned(),
        ),
        (ATTR_SERVICE.to_owned(), item.service().to_owned()),
        (ATTR_ACCOUNT.to_owned(), item.account().to_owned()),
    ])
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn exact_attributes_match(
    candidate: &BTreeMap<String, String>,
    expected: &BTreeMap<String, String>,
) -> bool {
    expected
        .iter()
        .all(|(key, value)| candidate.get(key) == Some(value))
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn read_with_backend(
    backend: &dyn SecretServiceBackend,
    item: &KeychainItem,
) -> Result<SecretToken, KeychainError> {
    let expected = item_attributes(item);
    let candidates = backend.search_items(&expected)?;
    let exact: Vec<&SecretRecord> = candidates
        .iter()
        .filter(|record| exact_attributes_match(&record.attributes, &expected))
        .collect();
    if exact.is_empty() {
        return Err(KeychainError::Missing);
    }
    if exact.iter().any(|record| record.locked) {
        return Err(KeychainError::LockedDenied);
    }
    let first = &exact[0].secret;
    if exact.iter().any(|record| record.secret != *first) {
        return Err(KeychainError::Unavailable);
    }
    let value = String::from_utf8(first.clone()).map_err(|_| KeychainError::Unavailable)?;
    SecretToken::new(value)
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn write_with_backend(
    backend: &dyn SecretServiceBackend,
    item: &KeychainItem,
    token: &SecretToken,
) -> Result<(), KeychainError> {
    let attrs = item_attributes(item);
    let label = format!("Fleetworks Native ({})", item.service());
    backend.write_item(&label, &attrs, token.expose_secret().as_bytes())
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn delete_with_backend(
    backend: &dyn SecretServiceBackend,
    item: &KeychainItem,
) -> Result<(), KeychainError> {
    let attrs = item_attributes(item);
    let removed = backend.delete_matching(&attrs)?;
    if removed == 0 {
        return Err(KeychainError::Missing);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
mod live {
    use std::collections::BTreeMap;
    use std::collections::HashMap;
    use std::future::Future;

    use secret_service::Collection;
    use secret_service::EncryptionType;
    use secret_service::Error as ServiceError;
    use secret_service::SecretService;

    use super::exact_attributes_match;
    use super::SecretRecord;
    use super::SecretServiceBackend;
    use crate::native::keychain::KeychainError;

    pub(crate) struct DbusSecretBackend;

    impl SecretServiceBackend for DbusSecretBackend {
        fn search_items(
            &self,
            attributes: &BTreeMap<String, String>,
        ) -> Result<Vec<SecretRecord>, KeychainError> {
            let owned = attributes.clone();
            block_on(async move {
                let service = SecretService::connect(EncryptionType::Dh)
                    .await
                    .map_err(map_service_error)?;
                let collection = unlocked_default_collection(&service).await?;
                let query: HashMap<&str, &str> = owned
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                let items = collection
                    .search_items(query)
                    .await
                    .map_err(map_service_error)?;
                let mut records = Vec::new();
                for item in items {
                    let attrs_map = item.get_attributes().await.map_err(map_service_error)?;
                    let attrs: BTreeMap<String, String> = attrs_map.into_iter().collect();
                    if !exact_attributes_match(&attrs, &owned) {
                        continue;
                    }
                    if item.is_locked().await.map_err(map_service_error)? {
                        item.unlock().await.map_err(map_service_error)?;
                    }
                    if item.is_locked().await.map_err(map_service_error)? {
                        return Err(KeychainError::LockedDenied);
                    }
                    let secret = item.get_secret().await.map_err(map_service_error)?;
                    records.push(SecretRecord {
                        attributes: attrs,
                        locked: false,
                        secret,
                    });
                }
                Ok(records)
            })
        }

        fn write_item(
            &self,
            label: &str,
            attributes: &BTreeMap<String, String>,
            secret: &[u8],
        ) -> Result<(), KeychainError> {
            let label = label.to_owned();
            let owned = attributes.clone();
            let secret = secret.to_vec();
            block_on(async move {
                let service = SecretService::connect(EncryptionType::Dh)
                    .await
                    .map_err(map_service_error)?;
                let collection = unlocked_default_collection(&service).await?;
                let query: HashMap<&str, &str> = owned
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                collection
                    .create_item(&label, query, &secret, true, "text/plain")
                    .await
                    .map_err(map_service_error)?;
                Ok(())
            })
        }

        fn delete_matching(
            &self,
            attributes: &BTreeMap<String, String>,
        ) -> Result<usize, KeychainError> {
            let owned = attributes.clone();
            block_on(async move {
                let service = SecretService::connect(EncryptionType::Dh)
                    .await
                    .map_err(map_service_error)?;
                let collection = unlocked_default_collection(&service).await?;
                let query: HashMap<&str, &str> = owned
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                let items = collection
                    .search_items(query)
                    .await
                    .map_err(map_service_error)?;
                let mut removed = 0usize;
                for item in items {
                    let attrs_map = item.get_attributes().await.map_err(map_service_error)?;
                    let attrs: BTreeMap<String, String> = attrs_map.into_iter().collect();
                    if !exact_attributes_match(&attrs, &owned) {
                        continue;
                    }
                    item.delete().await.map_err(map_service_error)?;
                    removed += 1;
                }
                Ok(removed)
            })
        }
    }

    async fn unlocked_default_collection<'a>(
        service: &'a SecretService<'a>,
    ) -> Result<Collection<'a>, KeychainError> {
        let collection = service
            .get_default_collection()
            .await
            .map_err(map_service_error)?;
        if collection.is_locked().await.map_err(map_service_error)? {
            collection.unlock().await.map_err(map_service_error)?;
        }
        if collection.is_locked().await.map_err(map_service_error)? {
            return Err(KeychainError::LockedDenied);
        }
        Ok(collection)
    }

    fn block_on<T>(
        future: impl Future<Output = Result<T, KeychainError>>,
    ) -> Result<T, KeychainError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| KeychainError::Unavailable)?;
        runtime.block_on(future)
    }

    pub(crate) fn map_service_error(error: ServiceError) -> KeychainError {
        match error {
            ServiceError::Locked | ServiceError::Prompt => KeychainError::LockedDenied,
            ServiceError::ZbusFdo(ref fdo) => match **fdo {
                zbus::fdo::Error::AccessDenied(_) | zbus::fdo::Error::AuthFailed(_) => {
                    KeychainError::LockedDenied
                }
                _ => KeychainError::Unavailable,
            },
            ServiceError::Zbus(zbus::Error::FDO(ref fdo)) => match **fdo {
                zbus::fdo::Error::AccessDenied(_) | zbus::fdo::Error::AuthFailed(_) => {
                    KeychainError::LockedDenied
                }
                _ => KeychainError::Unavailable,
            },
            ServiceError::Unavailable
            | ServiceError::NoResult
            | ServiceError::Crypto(_)
            | ServiceError::Zvariant(_)
            | ServiceError::Zbus(_) => KeychainError::Unavailable,
            _ => KeychainError::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakeDbusBackend {
        records: Mutex<Vec<SecretRecord>>,
        forced_error: Mutex<Option<KeychainError>>,
    }

    impl SecretServiceBackend for FakeDbusBackend {
        fn search_items(
            &self,
            _attributes: &BTreeMap<String, String>,
        ) -> Result<Vec<SecretRecord>, KeychainError> {
            if let Some(err) = *self.forced_error.lock().expect("lock") {
                return Err(err);
            }
            Ok(self.records.lock().expect("lock").clone())
        }

        fn write_item(
            &self,
            _label: &str,
            attributes: &BTreeMap<String, String>,
            secret: &[u8],
        ) -> Result<(), KeychainError> {
            if let Some(err) = *self.forced_error.lock().expect("lock") {
                return Err(err);
            }
            let mut records = self.records.lock().expect("lock");
            records.retain(|row| row.attributes != *attributes);
            records.push(SecretRecord {
                attributes: attributes.clone(),
                locked: false,
                secret: secret.to_vec(),
            });
            Ok(())
        }

        fn delete_matching(
            &self,
            attributes: &BTreeMap<String, String>,
        ) -> Result<usize, KeychainError> {
            if let Some(err) = *self.forced_error.lock().expect("lock") {
                return Err(err);
            }
            let mut records = self.records.lock().expect("lock");
            let before = records.len();
            records.retain(|row| !exact_attributes_match(&row.attributes, attributes));
            Ok(before - records.len())
        }
    }

    #[test]
    fn exact_attribute_matching_rejects_partial_or_other_origin_items() {
        let item_a = KeychainItem::new("inst-1", "https://a.example", "credential", "cred-1");
        let item_b = KeychainItem::new("inst-1", "https://b.example", "credential", "cred-1");
        let backend = FakeDbusBackend::default();
        let token_b = SecretToken::new("tok-secret-b".into()).expect("token");
        write_with_backend(&backend, &item_b, &token_b).expect("write b");

        // Partial/other origin must never satisfy lookup for item_a.
        assert_eq!(
            read_with_backend(&backend, &item_a),
            Err(KeychainError::Missing)
        );
        assert_eq!(
            delete_with_backend(&backend, &item_a),
            Err(KeychainError::Missing)
        );
        assert_eq!(read_with_backend(&backend, &item_b), Ok(token_b.clone()));
    }

    #[test]
    fn locked_and_unavailable_states_never_downgrade_to_missing() {
        let item = KeychainItem::new("inst-1", "https://a.example", "credential", "cred-1");
        let backend = FakeDbusBackend::default();

        *backend.forced_error.lock().expect("lock") = Some(KeychainError::LockedDenied);
        assert_eq!(
            read_with_backend(&backend, &item),
            Err(KeychainError::LockedDenied)
        );

        *backend.forced_error.lock().expect("lock") = Some(KeychainError::Unavailable);
        assert_eq!(
            read_with_backend(&backend, &item),
            Err(KeychainError::Unavailable)
        );

        // Even if backend returns a locked item directly without failing search, fail LockedDenied.
        *backend.forced_error.lock().expect("lock") = None;
        backend.records.lock().expect("lock").push(SecretRecord {
            attributes: item_attributes(&item),
            locked: true,
            secret: b"tok-secret".to_vec(),
        });
        assert_eq!(
            read_with_backend(&backend, &item),
            Err(KeychainError::LockedDenied)
        );
    }

    #[test]
    fn conflicting_duplicate_items_fail_closed() {
        let item = KeychainItem::new("inst-1", "https://a.example", "credential", "cred-1");
        let backend = FakeDbusBackend::default();
        let attrs = item_attributes(&item);
        backend.records.lock().expect("lock").extend([
            SecretRecord {
                attributes: attrs.clone(),
                locked: false,
                secret: b"secret-one".to_vec(),
            },
            SecretRecord {
                attributes: attrs,
                locked: false,
                secret: b"secret-two".to_vec(),
            },
        ]);
        assert_eq!(
            read_with_backend(&backend, &item),
            Err(KeychainError::Unavailable)
        );
    }
}
