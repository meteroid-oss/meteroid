pub mod webhook_security {
    use base64::Engine;
    use secrecy::SecretString;

    const KEY_SIZE: usize = 24;
    const PREFIX: &str = "whsec_";

    pub fn gen() -> SecretString {
        let key: Vec<u8> = std::iter::repeat_with(|| fastrand::u8(..))
            .take(KEY_SIZE)
            .collect();
        let encoded = base64::prelude::BASE64_STANDARD.encode(&key);

        SecretString::new(format!("{}{}", PREFIX, encoded))
    }
}
