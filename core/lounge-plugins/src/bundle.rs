//! `.lgp` bundles: a zip with `manifest.json`, `plugin.wasm`, `publisher.pub` (Ed25519 public
//! key, 32 bytes or 64 hex chars) and `signature.sig` (64 bytes or 128 hex chars) over
//! [`Bundle::signed_bytes`]. The publisher key is pinned on first install; an update whose key
//! differs is refused by the installer.

use crate::{api::Manifest, PluginError, Result};
use ed25519_dalek::{Signature, VerifyingKey};
use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8] = b"lounge-plugin-v1\0";

pub struct Bundle {
    pub manifest: Manifest,
    pub manifest_json: Vec<u8>,
    pub wasm: Vec<u8>,
    pub publisher: [u8; 32],
    pub signature: [u8; 64],
}

impl Bundle {
    pub fn read(path: &Path) -> Result<Bundle> {
        let file = std::fs::File::open(path)?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| PluginError::Bundle(e.to_string()))?;
        let manifest_json = entry(&mut zip, "manifest.json", 256 * 1024)?;
        let wasm = entry(&mut zip, "plugin.wasm", 16 * 1024 * 1024)?;
        let publisher = key_bytes::<32>(&entry(&mut zip, "publisher.pub", 256)?, "publisher.pub")?;
        let signature = key_bytes::<64>(&entry(&mut zip, "signature.sig", 256)?, "signature.sig")?;
        let manifest: Manifest = serde_json::from_slice(&manifest_json)?;
        manifest.validate().map_err(PluginError::Manifest)?;
        Ok(Bundle { manifest, manifest_json, wasm, publisher, signature })
    }

    /// Verify the Ed25519 signature over the manifest and module.
    pub fn verify(&self) -> Result<()> {
        let key = VerifyingKey::from_bytes(&self.publisher).map_err(|_| PluginError::Signature)?;
        let sig = Signature::from_bytes(&self.signature);
        // Strict: no weak keys or signatures that other verifiers would read differently.
        key.verify_strict(&Self::signed_bytes(&self.manifest_json, &self.wasm), &sig)
            .map_err(|_| PluginError::Signature)
    }

    /// The bytes a publisher signs: a magic prefix, then the length-prefixed manifest and module.
    pub fn signed_bytes(manifest_json: &[u8], wasm: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAGIC.len() + 16 + manifest_json.len() + wasm.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(manifest_json.len() as u64).to_le_bytes());
        out.extend_from_slice(manifest_json);
        out.extend_from_slice(&(wasm.len() as u64).to_le_bytes());
        out.extend_from_slice(wasm);
        out
    }

    pub fn publisher_hex(&self) -> String {
        hex::encode(self.publisher)
    }

    /// Write a bundle; used by the `lgp` tool.
    pub fn write(path: &Path, manifest_json: &[u8], wasm: &[u8], publisher: &[u8; 32], signature: &[u8; 64]) -> Result<()> {
        let file = std::fs::File::create(path)?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let z = |e: zip::result::ZipError| PluginError::Bundle(e.to_string());
        zip.start_file("manifest.json", opts).map_err(z)?;
        zip.write_all(manifest_json)?;
        zip.start_file("plugin.wasm", opts).map_err(z)?;
        zip.write_all(wasm)?;
        zip.start_file("publisher.pub", stored).map_err(z)?;
        zip.write_all(hex::encode(publisher).as_bytes())?;
        zip.start_file("signature.sig", stored).map_err(z)?;
        zip.write_all(hex::encode(signature).as_bytes())?;
        zip.finish().map_err(z)?;
        Ok(())
    }
}

fn entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str, max: usize) -> Result<Vec<u8>> {
    let mut f = zip
        .by_name(name)
        .map_err(|_| PluginError::Bundle(format!("{name} missing")))?;
    if f.size() as usize > max {
        return Err(PluginError::Bundle(format!("{name} too large")));
    }
    let mut buf = Vec::with_capacity((f.size() as usize).min(max));
    f.by_ref().take(max as u64 + 1).read_to_end(&mut buf)?;
    if buf.len() > max {
        return Err(PluginError::Bundle(format!("{name} too large")));
    }
    Ok(buf)
}

fn key_bytes<const N: usize>(raw: &[u8], what: &str) -> Result<[u8; N]> {
    let bytes: Vec<u8> = if raw.len() == N {
        raw.to_vec()
    } else {
        let text = std::str::from_utf8(raw).map_err(|_| PluginError::Bundle(format!("{what} malformed")))?;
        hex::decode(text.trim()).map_err(|_| PluginError::Bundle(format!("{what} malformed")))?
    };
    bytes.try_into().map_err(|_| PluginError::Bundle(format!("{what} has the wrong length")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn bundle_roundtrip_and_tamper() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.lgp");
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let manifest = br#"{"id":"tv.lounge.test","name":"Test","version":"1"}"#;
        let wasm = b"\0asm\x01\0\0\0";
        let sig = key.sign(&Bundle::signed_bytes(manifest, wasm));
        Bundle::write(&path, manifest, wasm, key.verifying_key().as_bytes(), &sig.to_bytes()).unwrap();
        let b = Bundle::read(&path).unwrap();
        assert_eq!(b.manifest.id, "tv.lounge.test");
        b.verify().unwrap();

        let mut tampered = Bundle::read(&path).unwrap();
        tampered.wasm.push(0);
        assert!(matches!(tampered.verify(), Err(PluginError::Signature)));
    }
}
