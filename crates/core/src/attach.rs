//! Attachments: content-addressed blobs in `.vault/blobs/<first 2 hex>/<sha256>`.
//! SHA-256 is implemented here (FIPS 180-4) to avoid a dependency.

use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use crate::error::{invalid, not_found, Result};

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Incremental SHA-256.
pub(crate) struct Sha256 {
    h: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total: u64,
}

impl Sha256 {
    pub fn new() -> Sha256 {
        Sha256 {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0; 64],
            buf_len: 0,
            total: 0,
        }
    }

    fn block(&mut self, b: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, c) in b.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b_, mut c, mut d, mut e, mut f, mut g, mut h] = self.h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b_) ^ (a & c) ^ (b_ & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b_;
            b_ = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.h.iter_mut().zip([a, b_, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total += data.len() as u64;
        if self.buf_len > 0 {
            let n = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + n].copy_from_slice(&data[..n]);
            self.buf_len += n;
            data = &data[n..];
            if self.buf_len == 64 {
                let b = self.buf;
                self.block(&b);
                self.buf_len = 0;
            }
        }
        while data.len() >= 64 {
            let b: [u8; 64] = data[..64].try_into().unwrap();
            self.block(&b);
            data = &data[64..];
        }
        self.buf[..data.len()].copy_from_slice(data);
        self.buf_len += data.len();
    }

    pub fn hex(mut self) -> String {
        let bits = self.total.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buf_len != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        self.h.iter().map(|x| format!("{x:08x}")).collect()
    }
}

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    h.hex()
}

pub(crate) fn is_attachment_id(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub(crate) fn blobs_dir(root: &Path) -> PathBuf {
    root.join(".vault").join("blobs")
}

pub(crate) fn blob_path(root: &Path, id: &str) -> PathBuf {
    blobs_dir(root).join(&id[..2]).join(id)
}

/// MIME type from the file name's extension.
pub(crate) fn mime_of(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "heic" => "image/heic",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "zip" => "application/zip",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
}

pub(crate) fn is_image(name: &str) -> bool {
    mime_of(name).starts_with("image/")
}

/// Stores bytes as a blob (no-op if already present). Returns (id, size).
pub(crate) fn store_bytes(root: &Path, bytes: &[u8]) -> Result<(String, i64)> {
    let id = sha256_hex(bytes);
    let path = blob_path(root, &id);
    if !path.exists() {
        fs::create_dir_all(path.parent().expect("blob dir"))?;
        crate::vault::write_atomic(&path, bytes)?;
    }
    Ok((id, bytes.len() as i64))
}

/// Copies a file into the blob store, hashing while copying. Returns (id, size).
pub(crate) fn store_file(root: &Path, src: &Path) -> Result<(String, i64)> {
    let mut f = match File::open(src) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(not_found("file", &src.to_string_lossy()))
        }
        Err(e) => return Err(e.into()),
    };
    if !f.metadata()?.is_file() {
        return Err(invalid(format!("{} is not a file", src.display())));
    }
    let dir = blobs_dir(root);
    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("incoming-{}.tmp", crate::ids::new_id()));
    let mut out = File::create(&tmp)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut size = 0i64;
    let res = (|| -> Result<()> {
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            out.write_all(&buf[..n])?;
            size += n as i64;
        }
        out.flush()?;
        Ok(())
    })();
    drop(out);
    if let Err(e) = res {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    let id = h.hex();
    let path = blob_path(root, &id);
    if path.exists() {
        fs::remove_file(&tmp)?;
    } else {
        fs::create_dir_all(path.parent().expect("blob dir"))?;
        fs::rename(&tmp, &path)?;
    }
    Ok((id, size))
}

/// Every blob on disk: (id, size).
pub(crate) fn list_blobs(root: &Path) -> Result<Vec<(String, i64)>> {
    let mut out = Vec::new();
    let dir = blobs_dir(root);
    let Ok(rd) = fs::read_dir(&dir) else {
        return Ok(out);
    };
    for sub in rd {
        let sub = sub?;
        if !sub.file_type()?.is_dir() {
            continue;
        }
        for e in fs::read_dir(sub.path())? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if is_attachment_id(&name) {
                out.push((name, e.metadata()?.len() as i64));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million = vec![b'a'; 1_000_000];
        let mut h = Sha256::new();
        for c in million.chunks(777) {
            h.update(c);
        }
        assert_eq!(
            h.hex(),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }
}
