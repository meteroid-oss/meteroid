//! Its own test binary, so no other test has installed a crypto provider in this process.

#[test]
fn builds_a_client_when_no_provider_is_installed() {
    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
    http_client_tracing::client_builder().build().unwrap();
    http_client_tracing::client();
}
