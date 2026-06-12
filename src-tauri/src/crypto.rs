use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("encryption failed")]
    EncryptFailed,
    #[error("decryption failed")]
    DecryptFailed,
    #[cfg(not(windows))]
    #[error("DPAPI is only supported on Windows")]
    NotSupported,
}

#[cfg(windows)]
mod windows_dpapi {
    use super::CryptoError;
    use windows::Win32::Foundation::LocalFree;
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    pub fn protect(plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: plaintext.len() as u32,
            pbData: plaintext.as_ptr() as *mut u8,
        };

        let mut output = CRYPT_INTEGER_BLOB::default();

        unsafe {
            CryptProtectData(
                &mut input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|_| CryptoError::EncryptFailed)?;

            let data =
                std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            let _ = LocalFree(windows::Win32::Foundation::HLOCAL(output.pbData as _));
            Ok(data)
        }
    }

    pub fn unprotect(ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: ciphertext.len() as u32,
            pbData: ciphertext.as_ptr() as *mut u8,
        };

        let mut output = CRYPT_INTEGER_BLOB::default();

        unsafe {
            CryptUnprotectData(
                &mut input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|_| CryptoError::DecryptFailed)?;

            let data =
                std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            let _ = LocalFree(windows::Win32::Foundation::HLOCAL(output.pbData as _));
            Ok(data)
        }
    }
}

pub fn encrypt_secret(plaintext: &str) -> Result<String, CryptoError> {
    #[cfg(windows)]
    {
        let encrypted = windows_dpapi::protect(plaintext.as_bytes())?;
        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            encrypted,
        ))
    }
    #[cfg(not(windows))]
    {
        let _ = plaintext;
        Err(CryptoError::NotSupported)
    }
}

pub fn decrypt_secret(encoded: &str) -> Result<String, CryptoError> {
    #[cfg(windows)]
    {
        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            encoded,
        )
        .map_err(|_| CryptoError::DecryptFailed)?;
        let decrypted = windows_dpapi::unprotect(&bytes)?;
        String::from_utf8(decrypted).map_err(|_| CryptoError::DecryptFailed)
    }
    #[cfg(not(windows))]
    {
        let _ = encoded;
        Err(CryptoError::NotSupported)
    }
}

pub fn mask_secret(secret: &str) -> String {
    if secret.len() <= 8 {
        return "••••••••".to_string();
    }
    let prefix = &secret[..4];
    let suffix = &secret[secret.len() - 4..];
    format!("{prefix}••••••••{suffix}")
}

/// OpenAI-compatible gateway keys so Cursor accepts and validates them.
pub fn generate_gateway_key() -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    let suffix: String = (0..40)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect();
    format!("sk-kimi-{suffix}")
}

pub fn is_openai_style_gateway_key(key: &str) -> bool {
    key.starts_with("sk-") && key.len() >= 20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_key_looks_like_openai_key() {
        let key = generate_gateway_key();
        assert!(key.starts_with("sk-kimi-"));
        assert!(super::is_openai_style_gateway_key(&key));
    }

    #[test]
    #[cfg(windows)]
    fn round_trip_secret() {
        let original = "sk-test-moonshot-key-12345";
        let encrypted = encrypt_secret(original).expect("encrypt");
        let decrypted = decrypt_secret(&encrypted).expect("decrypt");
        assert_eq!(original, decrypted);
    }

    #[test]
    fn mask_hides_middle() {
        let masked = mask_secret("sk-abcdefghijklmnop");
        assert!(masked.contains("••••"));
        assert!(!masked.contains("ijkl"));
    }
}
