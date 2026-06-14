use waft::identity::Identity;

#[test]
fn test_generate_unique() {
    let id1 = Identity::generate();
    let id2 = Identity::generate();
    assert_ne!(id1.fingerprint(), id2.fingerprint());
}

#[test]
fn test_save_and_load() -> Result<(), anyhow::Error> {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join("waft_test_identity");

    if file_path.exists() {
        let _ = std::fs::remove_file(&file_path);
    }

    let identity = Identity::generate();
    identity.save_to_file(&file_path)?;

    let loaded = Identity::load_from_file(&file_path)?;
    assert_eq!(identity.fingerprint(), loaded.fingerprint());

    let _ = std::fs::remove_file(&file_path);
    Ok(())
}

#[test]
fn test_sign_and_verify() {
    use ed25519_dalek::Verifier;

    let identity = Identity::generate();
    let message = b"waft cryptographic handshake validation";
    let signature = identity.sign(message);

    assert!(identity.public_key().verify(message, &signature).is_ok());
}
