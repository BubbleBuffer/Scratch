//! The zstd transport: the `zstd-v1` capability, negotiated per request, and
//! the compressed bodies it allows. Binary v1 ([`crate::wire`]) is unchanged;
//! a compressed body is the same v1 bytes, zstd-framed.
//!
//! A request advertises the capability with `X-Sync-Capability: zstd-v1`
//! (an exact token; anything else means none). A hub that sees it confirms
//! with the same header on its `/sync` and `/push` answers and may compress
//! those answers. A compressed body says so with `Content-Encoding: zstd`,
//! and is only ever read after the negotiation checks.

use std::io::Read;

use crate::{Result, SyncError, CAPABILITY_HEADER, ZSTD_CAPABILITY};

/// zstd level for compressed bodies.
const LEVEL: i32 = 3;
/// Bodies shorter than this are sent plain.
const MIN_LEN: usize = 1024;
/// Largest window (log2 bytes) a compressed body may ask the decoder for:
/// 8 MB. Level 3 needs 2 MB, so this only refuses frames built to exhaust
/// memory.
const WINDOW_LOG_MAX: u32 = 23;

/// The negotiation of one request, read from its headers before its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Negotiated {
    /// The request advertised `zstd-v1`: its answer may be compressed and
    /// carries the confirmation.
    pub advertised: bool,
    /// The request body is zstd-compressed.
    pub body_zstd: bool,
}

/// The negotiation an answer carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Answer {
    /// The hub confirmed `zstd-v1`.
    pub confirmed: bool,
    /// The body is zstd-compressed.
    pub zstd: bool,
}

/// Header values with the name (case-insensitive), trimmed of spaces.
fn values<'a>(headers: &'a [(String, String)], name: &str) -> Vec<&'a str> {
    headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim())
        .collect()
}

/// Whether the capability header is exactly `zstd-v1`, once. Another token,
/// or the header twice, means no capability.
fn advertises(headers: &[(String, String)]) -> bool {
    matches!(values(headers, CAPABILITY_HEADER).as_slice(), [v] if *v == ZSTD_CAPABILITY)
}

/// Reads a request's negotiation, before its body is read. A compressed body
/// is refused (415) unless the request advertised the capability, and any
/// other encoding is refused. `body_allowed` is false for bodies that are
/// never compressed (attachments).
pub(crate) fn request_negotiation(
    headers: &[(String, String)],
    body_allowed: bool,
) -> Result<Negotiated> {
    let advertised = advertises(headers);
    let body_zstd = match values(headers, "Content-Encoding").as_slice() {
        [] => false,
        [e] if e.eq_ignore_ascii_case("identity") => false,
        [e] if e.eq_ignore_ascii_case("zstd") && body_allowed => {
            if !advertised {
                return Err(SyncError::Status(
                    415,
                    "zstd body without the zstd-v1 capability".into(),
                ));
            }
            true
        }
        _ => {
            return Err(SyncError::Status(
                415,
                "unsupported content encoding".into(),
            ))
        }
    };
    Ok(Negotiated {
        advertised,
        body_zstd,
    })
}

/// Reads an answer's negotiation, before its body is decoded. A compressed
/// answer is an error unless the request advertised and the answer confirms.
pub(crate) fn answer_negotiation(headers: &[(String, String)], advertised: bool) -> Result<Answer> {
    let confirmed = advertises(headers);
    let zstd = match values(headers, "Content-Encoding").as_slice() {
        [] => false,
        [e] if e.eq_ignore_ascii_case("identity") => false,
        [e] if e.eq_ignore_ascii_case("zstd") => {
            if !(advertised && confirmed) {
                return Err(SyncError::Protocol(
                    "compressed answer without the zstd-v1 confirmation".into(),
                ));
            }
            true
        }
        _ => {
            return Err(SyncError::Protocol(
                "unsupported content encoding in answer".into(),
            ))
        }
    };
    Ok(Answer { confirmed, zstd })
}

/// The body as sent: compressed when that is worth it (`None` means send it
/// plain).
pub(crate) fn compress(raw: &[u8]) -> Result<Option<Vec<u8>>> {
    if raw.len() < MIN_LEN {
        return Ok(None);
    }
    let z =
        zstd::bulk::compress(raw, LEVEL).map_err(|e| SyncError::Io(format!("compress: {e}")))?;
    Ok((z.len() < raw.len()).then_some(z))
}

/// Decodes a compressed body into at most `limit` bytes, with the decoder's
/// window bounded. A body that decodes to more is refused (413); a malformed
/// one is a protocol error.
pub(crate) fn decompress(body: &[u8], limit: u64) -> Result<Vec<u8>> {
    let bad = |e: std::io::Error| SyncError::Protocol(format!("compressed body: {e}"));
    let mut dec = zstd::stream::read::Decoder::new(body).map_err(bad)?;
    dec.window_log_max(WINDOW_LOG_MAX).map_err(bad)?;
    let mut out = Vec::new();
    (&mut dec)
        .take(limit + 1)
        .read_to_end(&mut out)
        .map_err(bad)?;
    if out.len() as u64 > limit {
        return Err(SyncError::Status(413, "decoded body too large".into()));
    }
    Ok(out)
}

/// An answer's body and headers: the confirmation when the request
/// advertised, and the body compressed when that is worth it.
pub(crate) fn answer(body: Vec<u8>, advertised: bool) -> Result<(Vec<u8>, Vec<(String, String)>)> {
    if !advertised {
        return Ok((body, Vec::new()));
    }
    let mut headers = vec![(CAPABILITY_HEADER.to_string(), ZSTD_CAPABILITY.to_string())];
    match compress(&body)? {
        Some(z) => {
            headers.push(("Content-Encoding".to_string(), "zstd".to_string()));
            Ok((z, headers))
        }
        None => Ok((body, headers)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(name: &str, value: &str) -> (String, String) {
        (name.to_string(), value.to_string())
    }

    fn status(e: SyncError) -> u16 {
        match e {
            SyncError::Status(s, _) => s,
            e => panic!("not a status: {e}"),
        }
    }

    #[test]
    fn capability_is_an_exact_token() {
        let cap = CAPABILITY_HEADER;
        assert!(advertises(&[h(cap, "zstd-v1")]));
        assert!(advertises(&[h("x-sync-capability", " zstd-v1 ")]));
        assert!(!advertises(&[h(cap, "zstd-v2")]));
        assert!(!advertises(&[h(cap, "ZSTD-V1")]));
        assert!(!advertises(&[h(cap, "zstd-v1, zstd-v2")]));
        assert!(!advertises(&[h(cap, "zstd-v1"), h(cap, "zstd-v1")]));
        assert!(!advertises(&[]));
    }

    #[test]
    fn request_negotiation_checks_before_decoding() {
        let zstd = h("Content-Encoding", "zstd");
        let cap = h(CAPABILITY_HEADER, ZSTD_CAPABILITY);
        assert!(!request_negotiation(&[], true).unwrap().body_zstd);
        let n = request_negotiation(&[cap.clone(), zstd.clone()], true).unwrap();
        assert_eq!((n.advertised, n.body_zstd), (true, true));
        // Unnegotiated, unknown version, other encodings, attachments.
        assert_eq!(
            status(request_negotiation(&[zstd.clone()], true).unwrap_err()),
            415
        );
        let v2 = h(CAPABILITY_HEADER, "zstd-v2");
        assert_eq!(
            status(request_negotiation(&[v2, zstd.clone()], true).unwrap_err()),
            415
        );
        assert_eq!(
            status(
                request_negotiation(&[cap.clone(), h("Content-Encoding", "gzip")], true)
                    .unwrap_err()
            ),
            415
        );
        assert_eq!(
            status(
                request_negotiation(&[cap.clone(), zstd.clone(), zstd.clone()], true).unwrap_err()
            ),
            415
        );
        assert_eq!(
            status(request_negotiation(&[cap, zstd], false).unwrap_err()),
            415
        );
    }

    #[test]
    fn answers_need_the_confirmation() {
        let cap = h(CAPABILITY_HEADER, ZSTD_CAPABILITY);
        let zstd = h("Content-Encoding", "zstd");
        assert_eq!(
            answer_negotiation(&[cap.clone(), zstd.clone()], true).unwrap(),
            Answer {
                confirmed: true,
                zstd: true
            }
        );
        assert!(answer_negotiation(&[zstd.clone()], true).is_err());
        assert!(answer_negotiation(&[cap.clone(), zstd.clone()], false).is_err());
        assert!(answer_negotiation(&[h("Content-Encoding", "br")], true).is_err());
        let plain = answer_negotiation(&[], true).unwrap();
        assert_eq!(
            plain,
            Answer {
                confirmed: false,
                zstd: false
            }
        );
    }

    /// Deterministic pseudo-random bytes (xorshift64*): no repeats zstd can use.
    fn prng_bytes(n: usize) -> Vec<u8> {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        (0..n)
            .map(|_| {
                x ^= x >> 12;
                x ^= x << 25;
                x ^= x >> 27;
                (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
            })
            .collect()
    }

    #[test]
    fn small_or_unhelpful_bodies_stay_plain() {
        assert!(compress(&[7u8; 100]).unwrap().is_none());
        // Incompressible: the framed output is not smaller.
        let noise = prng_bytes(4096);
        assert!(compress(&noise).unwrap().is_none());
        let text = vec![b'a'; 64 << 10];
        let z = compress(&text).unwrap().expect("compressible");
        assert!(z.len() < 1024);
        assert_eq!(decompress(&z, 64 << 10).unwrap(), text);
    }

    #[test]
    fn decoded_size_is_bounded() {
        let text = vec![b'a'; 64 << 10];
        let z = compress(&text).unwrap().unwrap();
        assert_eq!(status(decompress(&z, 1024).unwrap_err()), 413);
        assert_eq!(decompress(&z, 64 << 10).unwrap().len(), 64 << 10);
    }

    #[test]
    fn garbage_is_a_protocol_error() {
        let e = decompress(b"not zstd at all", 1 << 20).unwrap_err();
        assert!(matches!(e, SyncError::Protocol(_)), "{e}");
        // A truncated frame is a decoder error, not a short read.
        let text = vec![b'b'; 64 << 10];
        let z = compress(&text).unwrap().unwrap();
        let e = decompress(&z[..z.len() / 2], 1 << 20).unwrap_err();
        assert!(matches!(e, SyncError::Protocol(_)), "{e}");
    }
}
