use std::fs;
use waft::trust::{TrustStore, TrustTier};

#[test]
fn test_unknown_peer_is_tier1() -> Result<(), anyhow::Error> {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("waft_test_trust_unknown");
    if file_path.exists() {
        fs::remove_file(&file_path)?;
    }

    let store = TrustStore::load_or_create(&file_path)?;
    assert_eq!(store.get_tier("fingerprint_unknown"), TrustTier::Ask);

    fs::remove_file(&file_path)?;
    Ok(())
}

#[test]
fn test_promote_and_get_tier() -> Result<(), anyhow::Error> {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("waft_test_trust_promote");
    if file_path.exists() {
        fs::remove_file(&file_path)?;
    }

    let store = TrustStore::load_or_create(&file_path)?;
    assert_eq!(store.get_tier("fingerprint_1"), TrustTier::Ask);

    store.set_tier("fingerprint_1", TrustTier::Trusted)?;
    assert_eq!(store.get_tier("fingerprint_1"), TrustTier::Trusted);

    store.set_tier("fingerprint_2", TrustTier::Blocked)?;
    assert_eq!(store.get_tier("fingerprint_2"), TrustTier::Blocked);

    fs::remove_file(&file_path)?;
    Ok(())
}

#[test]
fn test_trust_persists_across_restart() -> Result<(), anyhow::Error> {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("waft_test_trust_persist");
    if file_path.exists() {
        fs::remove_file(&file_path)?;
    }

    // Write to first instance of store
    {
        let store = TrustStore::load_or_create(&file_path)?;
        store.set_tier("fingerprint_a", TrustTier::Own)?;
        store.set_tier("fingerprint_b", TrustTier::Blocked)?;
    }

    // Load in second instance and verify values
    {
        let store = TrustStore::load_or_create(&file_path)?;
        assert_eq!(store.get_tier("fingerprint_a"), TrustTier::Own);
        assert_eq!(store.get_tier("fingerprint_b"), TrustTier::Blocked);
        assert_eq!(store.get_tier("fingerprint_c"), TrustTier::Ask);
    }

    fs::remove_file(&file_path)?;
    Ok(())
}

#[test]
fn test_malformed_toml_errors() -> Result<(), anyhow::Error> {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("waft_test_trust_malformed");

    // Write invalid content
    fs::write(
        &file_path,
        "peers = { fingerprint = 'invalid_format_because_not_a_map' }",
    )?;

    let store_result = TrustStore::load_or_create(&file_path);
    assert!(store_result.is_err());

    fs::remove_file(&file_path)?;
    Ok(())
}
