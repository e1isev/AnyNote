//! Content-addressed asset storage.
//!
//! Images, attachments and other binary files are stored once, named by the
//! BLAKE3 hash of their bytes (`assets/ab/abcdef...`). Documents reference
//! them as `anynote://asset/<hash>` so the same file pasted twice is stored
//! once, and exported folders can deduplicate too.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

pub const ASSET_URL_PREFIX: &str = "anynote://asset/";

#[derive(Debug, Clone)]
pub struct AssetStore {
    root: PathBuf,
}

impl AssetStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(AssetStore { root })
    }

    pub fn hash(bytes: &[u8]) -> String {
        blake3::hash(bytes).to_hex().to_string()
    }

    /// Store `bytes` and return their hash. Writing is atomic (temp file +
    /// rename), and a no-op when the asset already exists.
    pub fn put(&self, bytes: &[u8]) -> Result<String> {
        let hash = Self::hash(bytes);
        let path = self.path(&hash)?;
        if !path.exists() {
            let dir = path.parent().expect("asset path has a parent");
            fs::create_dir_all(dir)?;
            let tmp = dir.join(format!(".{hash}.tmp"));
            let mut f = fs::File::create(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            fs::rename(&tmp, &path)?;
        }
        Ok(hash)
    }

    pub fn get(&self, hash: &str) -> Result<Vec<u8>> {
        let path = self.path(hash)?;
        fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound(format!("asset {hash}")),
            _ => e.into(),
        })
    }

    pub fn path(&self, hash: &str) -> Result<PathBuf> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid(format!("bad asset hash {hash:?}")));
        }
        Ok(self.root.join(&hash[..2]).join(hash))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

pub fn asset_url(hash: &str) -> String {
    format!("{ASSET_URL_PREFIX}{hash}")
}

/// The hash in an `anynote://asset/<hash>` URL.
pub fn hash_from_url(url: &str) -> Option<&str> {
    url.strip_prefix(ASSET_URL_PREFIX).map(|h| h.split(['?', '#']).next().unwrap_or(h))
}

/// Best-effort MIME type from a file name.
pub fn mime_from_name(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "emf" => "image/emf",
        "wmf" => "image/wmf",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "mp4" => "video/mp4",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

/// Sniff a MIME type from the first bytes of a file.
pub fn sniff_mime(bytes: &[u8]) -> Option<&'static str> {
    Some(match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        [b'B', b'M', ..] => "image/bmp",
        [b'%', b'P', b'D', b'F', ..] => "application/pdf",
        [0x01, 0x00, 0x00, 0x00, ..] => "image/emf",
        [0xD7, 0xCD, 0xC6, 0x9A, ..] => "image/wmf",
        _ => return None,
    })
}

pub fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        "image/emf" => "emf",
        "image/wmf" => "wmf",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "application/json" => "json",
        "audio/mpeg" => "mp3",
        "audio/wav" => "wav",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let store = AssetStore::new(dir.path()).unwrap();
        let a = store.put(b"hello").unwrap();
        let b = store.put(b"hello").unwrap();
        assert_eq!(a, b);
        assert_eq!(store.get(&a).unwrap(), b"hello");
        assert!(store.path("../etc/passwd").is_err());
        assert_eq!(hash_from_url(&asset_url(&a)), Some(a.as_str()));
    }
}
