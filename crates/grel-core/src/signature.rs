//! Cryptographic signature verification (GPG and minisign).

use pgp::composed::Deserializable;

use crate::manifest::Manifest;
use crate::resolver::RemoteAsset;

/// Supported signature formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureKind {
    /// ASCII-armored OpenPGP detached signature.
    GpgAsc,
    /// Binary OpenPGP detached signature.
    GpgBinary,
    /// Minisign signature.
    Minisign,
}

/// Errors from signature operations.
#[derive(Debug, thiserror::Error)]
pub enum SignatureError {
    #[error("No signature file found for {0}")]
    NotFound(String),

    #[error("No trusted keys configured for {0} verification")]
    NoTrustedKeys(String),

    #[error("Invalid public key: {0}")]
    InvalidPublicKey(String),

    #[error("Invalid signature: {0}")]
    InvalidSignature(String),

    #[error("Signature verification failed: {0}")]
    VerificationFailed(String),
}

/// Locate and verify cryptographic signatures for release assets.
pub struct SignatureVerifier;

impl SignatureVerifier {
    /// Find a signature file for `target_asset` among `assets`.
    ///
    /// Preference:
    /// 1. Manifest `signature_filename` + `signature_kind`
    /// 2. `{filename}.minisig`
    /// 3. `{filename}.asc`
    /// 4. `{filename}.sig`
    pub fn find_signature_asset(
        assets: &[RemoteAsset],
        target_asset: &RemoteAsset,
        manifest: Option<&Manifest>,
    ) -> Option<(RemoteAsset, SignatureKind)> {
        // If the manifest specifies an exact signature file, try that first.
        if let Some(manifest) = manifest {
            if let Some(pattern) = &manifest.signature_filename {
                let candidate = Self::find_by_name(assets, pattern);
                if let Some(asset) = candidate {
                    let kind = manifest
                        .signature_kind
                        .as_deref()
                        .and_then(Self::parse_kind)
                        .unwrap_or_else(|| Self::infer_kind(&asset.filename));
                    return Some((asset, kind));
                }
            }
        }

        // Sidecar files, ordered by preference.
        let candidates = [
            (format!("{}.minisig", target_asset.filename), SignatureKind::Minisign),
            (format!("{}.asc", target_asset.filename), SignatureKind::GpgAsc),
            (format!("{}.sig", target_asset.filename), SignatureKind::GpgBinary),
        ];
        for (name, kind) in &candidates {
            if let Some(asset) = Self::find_by_name(assets, name) {
                return Some((asset, *kind));
            }
        }

        None
    }

    /// Verify a GPG detached signature.
    ///
    /// `public_keys` is a list of ASCII-armored or binary public keys.
    /// Returns `Ok` if **at least one** key verifies the signature.
    pub fn verify_gpg(
        signature_bytes: &[u8],
        message_bytes: &[u8],
        public_keys: &[String],
    ) -> Result<(), SignatureError> {
        if public_keys.is_empty() {
            return Err(SignatureError::NoTrustedKeys("GPG".into()));
        }

        // Parse the signature once.
        let sig = Self::parse_gpg_signature(signature_bytes)?;

        // Try each trusted key until one succeeds.
        for key_text in public_keys {
            let pubkey = match Self::parse_gpg_public_key(key_text.as_bytes()) {
                Ok(pk) => pk,
                Err(e) => {
                    tracing::debug!("Skipping invalid PGP key: {e}");
                    continue;
                }
            };

            if sig.verify(&pubkey, message_bytes).is_ok() {
                return Ok(());
            }
        }

        Err(SignatureError::VerificationFailed(
            "GPG signature did not verify against any trusted key".into(),
        ))
    }

    /// Verify a minisign signature.
    pub fn verify_minisign(
        signature_bytes: &[u8],
        message_bytes: &[u8],
        public_key_b64: &str,
    ) -> Result<(), SignatureError> {
        let public_key = minisign_verify::PublicKey::from_base64(public_key_b64)
            .map_err(|e| SignatureError::InvalidPublicKey(format!("{e}")))?;

        let signature = minisign_verify::Signature::decode(std::str::from_utf8(signature_bytes).unwrap_or(""))
            .map_err(|e| SignatureError::InvalidSignature(format!("{e}")))?;

        public_key
            .verify(message_bytes, &signature, false)
            .map_err(|e| SignatureError::VerificationFailed(format!("{e}")))?;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn find_by_name(assets: &[RemoteAsset], name: &str) -> Option<RemoteAsset> {
        assets.iter().find(|a| a.filename == name).cloned()
    }

    fn infer_kind(filename: &str) -> SignatureKind {
        let lower = filename.to_lowercase();
        if lower.ends_with(".minisig") {
            SignatureKind::Minisign
        } else if lower.ends_with(".asc") {
            SignatureKind::GpgAsc
        } else {
            SignatureKind::GpgBinary
        }
    }

    fn parse_kind(s: &str) -> Option<SignatureKind> {
        match s.to_lowercase().as_str() {
            "gpg" | "pgp" => Some(SignatureKind::GpgAsc),
            "minisign" | "signify" => Some(SignatureKind::Minisign),
            _ => None,
        }
    }

    fn parse_gpg_public_key(bytes: &[u8]) -> Result<pgp::composed::SignedPublicKey, SignatureError> {
        // Try ASCII-armored first, then binary.
        let str_input = std::str::from_utf8(bytes).unwrap_or("");
        if str_input.contains("-----BEGIN PGP PUBLIC KEY BLOCK-----") {
            let (pk, _headers) = pgp::composed::SignedPublicKey::from_string(str_input)
                .map_err(|e| SignatureError::InvalidPublicKey(format!("{e}")))?;
            Ok(pk)
        } else {
            let pk = pgp::composed::SignedPublicKey::from_bytes(bytes)
                .map_err(|e| SignatureError::InvalidPublicKey(format!("{e}")))?;
            Ok(pk)
        }
    }

    fn parse_gpg_signature(bytes: &[u8]) -> Result<pgp::composed::DetachedSignature, SignatureError> {
        let str_input = std::str::from_utf8(bytes).unwrap_or("");
        if str_input.contains("-----BEGIN PGP SIGNATURE-----") {
            let (sig, _headers) = pgp::composed::DetachedSignature::from_string(str_input)
                .map_err(|e| SignatureError::InvalidSignature(format!("{e}")))?;
            Ok(sig)
        } else {
            let sig = pgp::composed::DetachedSignature::from_bytes(bytes)
                .map_err(|e| SignatureError::InvalidSignature(format!("{e}")))?;
            Ok(sig)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_asset(filename: &str) -> RemoteAsset {
        RemoteAsset {
            filename: filename.to_string(),
            url: format!("https://example.com/{}", filename),
            size_bytes: None,
            tokens: crate::AssetTokens::from_filename(filename),
        }
    }

    // -----------------------------------------------------------------------
    // find_signature_asset tests
    // -----------------------------------------------------------------------

    #[test]
    fn find_asc_sidecar() {
        let target = make_asset("foo.tar.gz");
        let assets = vec![target.clone(), make_asset("foo.tar.gz.asc")];
        let result = SignatureVerifier::find_signature_asset(&assets, &target, None);
        assert!(result.is_some());
        let (asset, kind) = result.unwrap();
        assert_eq!(asset.filename, "foo.tar.gz.asc");
        assert_eq!(kind, SignatureKind::GpgAsc);
    }

    #[test]
    fn find_minisig_sidecar() {
        let target = make_asset("foo.tar.gz");
        let assets = vec![target.clone(), make_asset("foo.tar.gz.minisig")];
        let result = SignatureVerifier::find_signature_asset(&assets, &target, None);
        assert!(result.is_some());
        let (_, kind) = result.unwrap();
        assert_eq!(kind, SignatureKind::Minisign);
    }

    #[test]
    fn find_prefers_minisig_over_asc() {
        let target = make_asset("foo.tar.gz");
        let assets = vec![
            target.clone(),
            make_asset("foo.tar.gz.asc"),
            make_asset("foo.tar.gz.minisig"),
        ];
        let result = SignatureVerifier::find_signature_asset(&assets, &target, None);
        assert_eq!(result.unwrap().0.filename, "foo.tar.gz.minisig");
    }

    #[test]
    fn find_none_when_missing() {
        let target = make_asset("foo.tar.gz");
        let assets = vec![target.clone()];
        let result = SignatureVerifier::find_signature_asset(&assets, &target, None);
        assert!(result.is_none());
    }

    #[test]
    fn find_manifest_override() {
        let target = make_asset("foo.tar.gz");
        let assets = vec![
            target.clone(),
            make_asset("foo.tar.gz.asc"),
            make_asset("custom.sig"),
        ];
        let manifest = Manifest {
            name: String::new(),
            description: String::new(),
            license: String::new(),
            source: crate::manifest::SourceSpec::default(),
            assets: Vec::new(),
            checksum_filename: None,
            signature_filename: Some("custom.sig".into()),
            signature_kind: Some("gpg".into()),
            dependencies: crate::manifest::DependencySpec::default(),
            hooks: crate::manifest::HookSpec::default(),
        };
        let result = SignatureVerifier::find_signature_asset(&assets, &target, Some(&manifest));
        assert_eq!(result.unwrap().0.filename, "custom.sig");
    }

    // -----------------------------------------------------------------------
    // GPG verification tests
    // -----------------------------------------------------------------------

    /// Test Ed25519 public key generated with GnuPG 2.4.0.
    const TEST_GPG_PUBKEY: &str = r#"-----BEGIN PGP PUBLIC KEY BLOCK-----

mDMEag6EqxYJKwYBBAHaRw8BAQdAagHZINjf7b8TBhZ8LEuLgXw9cefg+pigLykJ
HT4WOQK0FWdyZWwtdGVzdEBleGFtcGxlLmNvbYiZBBMWCgBBFiEE8G3kCoSMwdHF
vZEJyISWM0hKoRoFAmoOhKsCGwMFCQPCZwAFCwkIBwICIgIGFQoJCAsCBBYCAwEC
HgcCF4AACgkQyISWM0hKoRqcWAD/YalmznHB4aKRfoCZiXtaThi5d/1nfvMq2QOT
pP1PwjABAN/SBfy6G/MWAAiQYYUIXdNH8Z3x3wNWt5diVrJTf7gJ
=xPA5
-----END PGP PUBLIC KEY BLOCK-----"#;

    /// Detached signature for the message "hello world".
    const TEST_GPG_SIGNATURE: &str = r#"-----BEGIN PGP SIGNATURE-----

iHUEABYKAB0WIQTwbeQKhIzB0cW9kQnIhJYzSEqhGgUCag6EqwAKCRDIhJYzSEqh
Gm2XAQC6qNb4Ur5uk/J7wRE5QAtI949Z97y2FY0cv+14IDmLggD/RcXkwL9E5U1g
zpEXEiW+ZNRF6dRDcLsW0V7Zc6VMiA0=
=1sC6
-----END PGP SIGNATURE-----"#;

    #[test]
    fn verify_gpg_valid_signature() {
        let message = b"hello world";
        let result = SignatureVerifier::verify_gpg(
            TEST_GPG_SIGNATURE.as_bytes(),
            message,
            &[TEST_GPG_PUBKEY.into()],
        );
        assert!(result.is_ok(), "Expected valid signature to verify: {:?}", result.err());
    }

    #[test]
    fn verify_gpg_tampered_message_fails() {
        let message = b"tampered message";
        let result = SignatureVerifier::verify_gpg(
            TEST_GPG_SIGNATURE.as_bytes(),
            message,
            &[TEST_GPG_PUBKEY.into()],
        );
        assert!(result.is_err());
    }

    #[test]
    fn verify_gpg_wrong_key_fails() {
        let wrong_key = r#"-----BEGIN PGP PUBLIC KEY BLOCK-----

mDMEZqQ2xBYJKwYBBAHaRw8BAQdA6r3ANcJP6EV3xYZ7VQqP9GQQesuvyQrCB8pG
uADBu560FHRlc3Qtd3JvbmdAaW52YWxpZC5jb22IkwQTFgoAOxYhBNAqI7nW9w1W
NXPulBzjxIZiIuRuBQJmpDbEAhsDBQsJCAcCAiICBhUKCQgLAgQWAgMBAh4HAheA
AAoJEBzjxIZiIuRuq80A/3n0rF6s5LZpF0bJEXAMPLEFAKEKEY000000000000
=FAKE
-----END PGP PUBLIC KEY BLOCK-----"#;

        let result = SignatureVerifier::verify_gpg(
            TEST_GPG_SIGNATURE.as_bytes(),
            b"hello world",
            &[wrong_key.into()],
        );
        assert!(result.is_err());
    }

    #[test]
    fn verify_gpg_no_keys_errors() {
        let result = SignatureVerifier::verify_gpg(
            TEST_GPG_SIGNATURE.as_bytes(),
            b"hello world",
            &[],
        );
        assert!(matches!(result, Err(SignatureError::NoTrustedKeys(_))));
    }

    // -----------------------------------------------------------------------
    // Minisign verification tests
    // -----------------------------------------------------------------------

    /// Test minisign public key generated with rsign2.
    const TEST_MINISIGN_PK: &str = "RWSLCRKwjvg2YcJc3ZmD/nJ1i6t+c75fHr4NaHdRBXfozl6ef5raBC29";

    /// Detached signature for the message "test".
    const TEST_MINISIGN_SIG: &str = r"untrusted comment: signature from rsign secret key
RUSLCRKwjvg2YX60MgzKVBdrV7qrLMFjLX0AVjQROiL4OoMFQJYs4OUbiYANqK6hcZtYVbZNvcZBbqEnEKF3lJac8Eh0q8LOLgw=
trusted comment: timestamp:1779336565	file:C:\Users\winnie\AppData\Local\Temp\grel-minisign-test\message.txt	prehashed
cuGMvy0qcsyzOF9xTD7XH0leSlH/tShnDSV3T8qHApjCIfMUg51Ab5gILEukbs24W4vop4iCXrR2ohLuBAsdDg==";

    #[test]
    fn verify_minisign_valid_signature() {
        let result = SignatureVerifier::verify_minisign(
            TEST_MINISIGN_SIG.as_bytes(),
            b"test",
            TEST_MINISIGN_PK,
        );
        assert!(result.is_ok(), "Expected valid minisign signature: {:?}", result.err());
    }

    #[test]
    fn verify_minisign_tampered_message_fails() {
        let result = SignatureVerifier::verify_minisign(
            TEST_MINISIGN_SIG.as_bytes(),
            b"tampered",
            TEST_MINISIGN_PK,
        );
        assert!(result.is_err());
    }

    #[test]
    fn verify_minisign_wrong_key_fails() {
        let result = SignatureVerifier::verify_minisign(
            TEST_MINISIGN_SIG.as_bytes(),
            b"test",
            "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO4", // last char changed
        );
        assert!(result.is_err());
    }
}
