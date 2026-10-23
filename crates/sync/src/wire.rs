//! The binary encoding of the protocol's request and response bodies:
//! a version byte, then fields in a fixed order. Integers are LEB128
//! varints; byte strings and text are a varint length and the bytes; lists
//! are a varint count and the items.

use crate::SyncError;
use vault_core::{PurgeRecord, RestoreRecord};

/// Bumped on any incompatible change to a message.
pub const PROTOCOL: u8 = 1;

/// The envelope version: a complete v1 message, then purge and restore
/// records. Only the `*V2` messages carry it.
pub const PURGE_PROTOCOL: u8 = 2;

/// (page id, bytes): a version vector or Loro updates.
pub type PageBytes = Vec<(String, Vec<u8>)>;

#[derive(Default)]
pub(crate) struct W(pub Vec<u8>);

impl W {
    pub fn new() -> W {
        W(vec![PROTOCOL])
    }
    /// An empty v2 envelope.
    pub fn v2() -> W {
        W(vec![PURGE_PROTOCOL])
    }
    pub fn var(&mut self, mut v: u64) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.0.push(b);
                return;
            }
            self.0.push(b | 0x80);
        }
    }
    pub fn bool(&mut self, v: bool) {
        self.0.push(v as u8);
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.var(b.len() as u64);
        self.0.extend_from_slice(b);
    }
    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    pub fn strs(&mut self, v: &[String]) {
        self.var(v.len() as u64);
        for s in v {
            self.str(s);
        }
    }
    pub fn pages(&mut self, v: &[(String, Vec<u8>)]) {
        self.var(v.len() as u64);
        for (id, b) in v {
            self.str(id);
            self.bytes(b);
        }
    }
    /// A presence flag, then the bytes when present (so absent and empty
    /// differ).
    pub fn opt(&mut self, v: Option<&[u8]>) {
        self.bool(v.is_some());
        if let Some(b) = v {
            self.bytes(b);
        }
    }
    /// Purge records. With `device` (the purge device capability, see
    /// [`crate::PURGE_DEVICE_CAPABILITY`]) each record also carries its
    /// device id: a presence byte, then the length-prefixed text when present.
    /// Without it the layout is the fieldless one, and `device` is dropped.
    pub fn purges(&mut self, v: &[PurgeRecord], device: bool) {
        self.var(v.len() as u64);
        for p in v {
            self.str(&p.page);
            self.opt(p.version.as_deref());
            if device {
                self.opt(p.device.as_deref().map(str::as_bytes));
            }
        }
    }
    pub fn restores(&mut self, v: &[RestoreRecord]) {
        self.var(v.len() as u64);
        for r in v {
            self.str(&r.page);
            self.opt(r.purge_version.as_deref());
            self.bytes(&r.version);
        }
    }
}

pub(crate) struct R<'a> {
    b: &'a [u8],
    i: usize,
}

fn bad(what: &str) -> SyncError {
    SyncError::Protocol(format!("malformed message: {what}"))
}

impl<'a> R<'a> {
    pub fn new(b: &'a [u8]) -> Result<R<'a>, SyncError> {
        Self::with_version(b, PROTOCOL)
    }
    /// A reader for a v2 envelope (first byte [`PURGE_PROTOCOL`]).
    pub fn new_v2(b: &'a [u8]) -> Result<R<'a>, SyncError> {
        Self::with_version(b, PURGE_PROTOCOL)
    }
    fn with_version(b: &'a [u8], version: u8) -> Result<R<'a>, SyncError> {
        match b.first() {
            Some(&v) if v == version => Ok(R { b, i: 1 }),
            Some(v) => Err(SyncError::Protocol(format!(
                "protocol version {v}, expected {version}"
            ))),
            None => Err(bad("empty")),
        }
    }
    /// A length-prefixed byte string, borrowed from the message.
    pub fn chunk(&mut self) -> Result<&'a [u8], SyncError> {
        let b: &'a [u8] = self.b;
        let n = usize::try_from(self.var()?).map_err(|_| bad("truncated"))?;
        if n > b.len() - self.i {
            return Err(bad("truncated"));
        }
        let v = &b[self.i..self.i + n];
        self.i += n;
        Ok(v)
    }
    /// A presence flag: exactly one byte, 0 or 1.
    fn flag(&mut self) -> Result<bool, SyncError> {
        let v = *self.b.get(self.i).ok_or_else(|| bad("truncated"))?;
        self.i += 1;
        match v {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(bad("flag is not 0 or 1")),
        }
    }
    fn opt(&mut self) -> Result<Option<Vec<u8>>, SyncError> {
        Ok(if self.flag()? {
            Some(self.bytes()?)
        } else {
            None
        })
    }
    /// Refuse impossible counts before allocating. The transport already
    /// bounds the body size; there is no separate lifetime tombstone limit.
    fn records(&mut self, minimum_bytes: usize) -> Result<usize, SyncError> {
        let n = self.count()?;
        if n > (self.b.len() - self.i) / minimum_bytes {
            return Err(bad("record count exceeds remaining bytes"));
        }
        Ok(n)
    }
    /// Purge records in the layout [`W::purges`] wrote with the same `device`.
    pub fn purges(&mut self, device: bool) -> Result<Vec<PurgeRecord>, SyncError> {
        // A record is at least a page length byte and one flag per field.
        let n = self.records(if device { 3 } else { 2 })?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            let page = self.str()?;
            let version = self.opt()?;
            let device = if device { self.text_opt()? } else { None };
            v.push(PurgeRecord {
                page,
                version,
                device,
            });
        }
        Ok(v)
    }
    /// A presence byte, then the length-prefixed UTF-8 text when present.
    fn text_opt(&mut self) -> Result<Option<String>, SyncError> {
        Ok(if self.flag()? {
            Some(self.str()?)
        } else {
            None
        })
    }
    pub fn restores(&mut self) -> Result<Vec<RestoreRecord>, SyncError> {
        let n = self.records(3)?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            let page = self.str()?;
            let purge_version = self.opt()?;
            let version = self.bytes()?;
            v.push(RestoreRecord {
                page,
                purge_version,
                version,
            });
        }
        Ok(v)
    }
    pub fn var(&mut self) -> Result<u64, SyncError> {
        let mut v: u64 = 0;
        for shift in (0..64).step_by(7) {
            let b = *self.b.get(self.i).ok_or_else(|| bad("truncated"))?;
            self.i += 1;
            v |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(bad("varint too long"))
    }
    pub fn u32(&mut self) -> Result<u32, SyncError> {
        u32::try_from(self.var()?).map_err(|_| bad("number too big"))
    }
    pub fn bool(&mut self) -> Result<bool, SyncError> {
        Ok(self.var()? != 0)
    }
    pub fn bytes(&mut self) -> Result<Vec<u8>, SyncError> {
        let n = self.var()? as usize;
        if n > self.b.len() - self.i {
            return Err(bad("truncated"));
        }
        let v = self.b[self.i..self.i + n].to_vec();
        self.i += n;
        Ok(v)
    }
    pub fn str(&mut self) -> Result<String, SyncError> {
        String::from_utf8(self.bytes()?).map_err(|_| bad("text is not UTF-8"))
    }
    /// A list count, checked against the bytes left (each item takes at
    /// least one byte), so a bad count cannot allocate much.
    fn count(&mut self) -> Result<usize, SyncError> {
        let n = self.var()? as usize;
        if n > self.b.len() - self.i {
            return Err(bad("list too long"));
        }
        Ok(n)
    }
    pub fn strs(&mut self) -> Result<Vec<String>, SyncError> {
        let n = self.count()?;
        (0..n).map(|_| self.str()).collect()
    }
    pub fn pages(&mut self) -> Result<PageBytes, SyncError> {
        let n = self.count()?;
        (0..n).map(|_| Ok((self.str()?, self.bytes()?))).collect()
    }
    pub fn end(&self) -> Result<(), SyncError> {
        if self.i == self.b.len() {
            Ok(())
        } else {
            Err(bad("trailing bytes"))
        }
    }
}

/// `POST /sync`: the client's versions; the hub answers with what the
/// client lacks and what it needs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncRequest {
    /// `versions` covers every page of the client (else only pages changed
    /// on the client since `base`).
    pub full: bool,
    /// The hub (epoch, seq) of the client's last complete sync.
    pub base: Option<(u64, u32)>,
    /// (page, encoded version vector); empty bytes for a page the client
    /// does not have.
    pub versions: PageBytes,
    /// Pages purged on the client (all, or new since the last sync).
    pub purged: Vec<String>,
    /// How many of the hub's purges the client has applied.
    pub hub_purged_known: u32,
    /// Attachments the client has that the hub may lack.
    pub blobs: Vec<String>,
    /// Most bytes of updates to put in the response (at least one page is
    /// always sent).
    pub budget: u64,
}

/// The hub's answer to [`SyncRequest`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncResponse {
    /// Random per hub run; a new epoch means the client's base is stale.
    pub epoch: u64,
    /// The hub's change feed position when the request was handled.
    pub seq: u32,
    /// The base is unusable (new epoch, or too old): sync in full.
    pub reset: bool,
    /// (page, updates) the client lacks.
    pub updates: PageBytes,
    /// (page, hub version) for each page in `updates`: the version the
    /// updates bring the client up to.
    pub versions: PageBytes,
    /// (page, hub version) where the client has changes the hub lacks:
    /// the client pushes `export_updates(page, hub version)`.
    pub need: PageBytes,
    /// (page, hub version) of pages changed on the hub since the base that
    /// the request did not cover; the client asks for those it lacks.
    pub changed: PageBytes,
    /// Hub purges the client has not applied yet.
    pub purged: Vec<String>,
    /// The hub's purge count (the client's next `hub_purged_known`).
    pub purged_total: u32,
    /// Attachments from `blobs` the hub lacks; the client uploads them.
    pub need_blobs: Vec<String>,
    /// The budget cut `updates` short: ask again.
    pub more: bool,
}

/// `POST /push`: updates the hub asked for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PushRequest {
    pub updates: PageBytes,
}

/// What a push did on the hub.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushResponse {
    pub created: u32,
    pub changed: u32,
    pub conflicts: u32,
}

/// `GET /changes`: the hub's change feed position.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChangesResponse {
    pub epoch: u64,
    pub seq: u32,
}

impl SyncRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::new();
        w.bool(self.full);
        match self.base {
            Some((e, s)) => {
                w.bool(true);
                w.var(e);
                w.var(s as u64);
            }
            None => w.bool(false),
        }
        w.pages(&self.versions);
        w.strs(&self.purged);
        w.var(self.hub_purged_known as u64);
        w.strs(&self.blobs);
        w.var(self.budget);
        w.0
    }
    pub fn decode(b: &[u8]) -> Result<SyncRequest, SyncError> {
        let mut r = R::new(b)?;
        let full = r.bool()?;
        let base = if r.bool()? {
            Some((r.var()?, r.u32()?))
        } else {
            None
        };
        let v = SyncRequest {
            full,
            base,
            versions: r.pages()?,
            purged: r.strs()?,
            hub_purged_known: r.u32()?,
            blobs: r.strs()?,
            budget: r.var()?,
        };
        r.end()?;
        Ok(v)
    }
}

impl SyncResponse {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::new();
        w.var(self.epoch);
        w.var(self.seq as u64);
        w.bool(self.reset);
        w.pages(&self.updates);
        w.pages(&self.versions);
        w.pages(&self.need);
        w.pages(&self.changed);
        w.strs(&self.purged);
        w.var(self.purged_total as u64);
        w.strs(&self.need_blobs);
        w.bool(self.more);
        w.0
    }
    pub fn decode(b: &[u8]) -> Result<SyncResponse, SyncError> {
        let mut r = R::new(b)?;
        let v = SyncResponse {
            epoch: r.var()?,
            seq: r.u32()?,
            reset: r.bool()?,
            updates: r.pages()?,
            versions: r.pages()?,
            need: r.pages()?,
            changed: r.pages()?,
            purged: r.strs()?,
            purged_total: r.u32()?,
            need_blobs: r.strs()?,
            more: r.bool()?,
        };
        r.end()?;
        Ok(v)
    }
}

impl PushRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::new();
        w.pages(&self.updates);
        w.0
    }
    pub fn decode(b: &[u8]) -> Result<PushRequest, SyncError> {
        let mut r = R::new(b)?;
        let v = PushRequest {
            updates: r.pages()?,
        };
        r.end()?;
        Ok(v)
    }
}

impl PushResponse {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::new();
        w.var(self.created as u64);
        w.var(self.changed as u64);
        w.var(self.conflicts as u64);
        w.0
    }
    pub fn decode(b: &[u8]) -> Result<PushResponse, SyncError> {
        let mut r = R::new(b)?;
        let v = PushResponse {
            created: r.u32()?,
            changed: r.u32()?,
            conflicts: r.u32()?,
        };
        r.end()?;
        Ok(v)
    }
}

impl ChangesResponse {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::new();
        w.var(self.epoch);
        w.var(self.seq as u64);
        w.0
    }
    pub fn decode(b: &[u8]) -> Result<ChangesResponse, SyncError> {
        let mut r = R::new(b)?;
        let v = ChangesResponse {
            epoch: r.var()?,
            seq: r.u32()?,
        };
        r.end()?;
        Ok(v)
    }
}

/// `POST /sync` for protocol v2: the v1 request plus the client's purge and
/// restore records.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncRequestV2 {
    pub request: SyncRequest,
    pub purges: Vec<PurgeRecord>,
    pub restores: Vec<RestoreRecord>,
}

/// The hub's answer to [`SyncRequestV2`], with its purge and restore
/// records.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SyncResponseV2 {
    pub response: SyncResponse,
    pub purges: Vec<PurgeRecord>,
    pub restores: Vec<RestoreRecord>,
}

/// `POST /push` for protocol v2: the v1 push plus the restore records.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PushRequestV2 {
    pub request: PushRequest,
    pub restores: Vec<RestoreRecord>,
}

impl SyncRequestV2 {
    /// The fieldless layout: purge records without device ids.
    pub fn encode(&self) -> Vec<u8> {
        self.encode_with_device(false)
    }
    /// With `device` (the purge device capability was confirmed), the purge
    /// records carry their device ids; otherwise this is [`Self::encode`].
    pub fn encode_with_device(&self, device: bool) -> Vec<u8> {
        let mut w = W::v2();
        w.bytes(&self.request.encode());
        w.purges(&self.purges, device);
        w.restores(&self.restores);
        w.0
    }
    pub fn decode(b: &[u8]) -> Result<SyncRequestV2, SyncError> {
        Self::decode_with_device(b, false)
    }
    /// Reads the layout [`Self::encode_with_device`] wrote with the same
    /// `device`. The other layout is refused, not reinterpreted.
    pub fn decode_with_device(b: &[u8], device: bool) -> Result<SyncRequestV2, SyncError> {
        let mut r = R::new_v2(b)?;
        let request = SyncRequest::decode(r.chunk()?)?;
        let purges = r.purges(device)?;
        let restores = r.restores()?;
        r.end()?;
        Ok(SyncRequestV2 {
            request,
            purges,
            restores,
        })
    }
}

impl SyncResponseV2 {
    /// The fieldless layout: purge records without device ids.
    pub fn encode(&self) -> Vec<u8> {
        self.encode_with_device(false)
    }
    /// See [`SyncRequestV2::encode_with_device`].
    pub fn encode_with_device(&self, device: bool) -> Vec<u8> {
        let mut w = W::v2();
        w.bytes(&self.response.encode());
        w.purges(&self.purges, device);
        w.restores(&self.restores);
        w.0
    }
    pub fn decode(b: &[u8]) -> Result<SyncResponseV2, SyncError> {
        Self::decode_with_device(b, false)
    }
    /// See [`SyncRequestV2::decode_with_device`].
    pub fn decode_with_device(b: &[u8], device: bool) -> Result<SyncResponseV2, SyncError> {
        let mut r = R::new_v2(b)?;
        let response = SyncResponse::decode(r.chunk()?)?;
        let purges = r.purges(device)?;
        let restores = r.restores()?;
        r.end()?;
        Ok(SyncResponseV2 {
            response,
            purges,
            restores,
        })
    }
}

impl PushRequestV2 {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::v2();
        w.bytes(&self.request.encode());
        w.restores(&self.restores);
        w.0
    }
    pub fn decode(b: &[u8]) -> Result<PushRequestV2, SyncError> {
        let mut r = R::new_v2(b)?;
        let request = PushRequest::decode(r.chunk()?)?;
        let restores = r.restores()?;
        r.end()?;
        Ok(PushRequestV2 { request, restores })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let req = SyncRequest {
            full: false,
            base: Some((u64::MAX, 7)),
            versions: vec![("p1".into(), vec![1, 2, 3]), ("p2".into(), vec![])],
            purged: vec!["gone".into()],
            hub_purged_known: 3,
            blobs: vec!["ab".repeat(32)],
            budget: 8 << 20,
        };
        assert_eq!(SyncRequest::decode(&req.encode()).unwrap(), req);
        let resp = SyncResponse {
            epoch: 42,
            seq: 300,
            reset: false,
            updates: vec![("p".into(), vec![0; 1000])],
            versions: vec![("p".into(), vec![4])],
            need: vec![("q".into(), vec![9])],
            changed: vec![],
            purged: vec!["x".into(), "ÿ".into()],
            purged_total: 2,
            need_blobs: vec![],
            more: true,
        };
        assert_eq!(SyncResponse::decode(&resp.encode()).unwrap(), resp);
        let push = PushRequest {
            updates: vec![("p".into(), vec![5; 3])],
        };
        assert_eq!(PushRequest::decode(&push.encode()).unwrap(), push);
        let pr = PushResponse {
            created: 1,
            changed: 2,
            conflicts: 3,
        };
        assert_eq!(PushResponse::decode(&pr.encode()).unwrap(), pr);
        let c = ChangesResponse { epoch: 1, seq: 2 };
        assert_eq!(ChangesResponse::decode(&c.encode()).unwrap(), c);
    }

    #[test]
    fn malformed_messages_are_errors() {
        let req = SyncRequest::default().encode();
        assert!(SyncRequest::decode(&req[..req.len() - 1]).is_err());
        let mut long = req.clone();
        long.push(0);
        assert!(SyncRequest::decode(&long).is_err());
        let mut other = req.clone();
        other[0] = PROTOCOL + 1;
        assert!(SyncRequest::decode(&other).is_err());
        assert!(SyncRequest::decode(&[]).is_err());
        // A list count far beyond the message does not allocate.
        let mut w = W::new();
        w.var(u64::MAX >> 1);
        assert!(PushRequest::decode(&w.0).is_err());
        let mut w = W::new();
        w.var(1);
        w.var(1 << 40);
        assert!(PushRequest::decode(&w.0).is_err());
    }

    fn purge(page: &str, version: Option<&[u8]>) -> PurgeRecord {
        PurgeRecord {
            page: page.into(),
            version: version.map(<[u8]>::to_vec),
            device: None,
        }
    }

    fn purge_from(page: &str, version: Option<&[u8]>, device: Option<&str>) -> PurgeRecord {
        PurgeRecord {
            device: device.map(str::to_string),
            ..purge(page, version)
        }
    }

    fn restore(page: &str, purge_version: Option<&[u8]>, version: &[u8]) -> RestoreRecord {
        RestoreRecord {
            page: page.into(),
            purge_version: purge_version.map(<[u8]>::to_vec),
            version: version.to_vec(),
        }
    }

    fn push_v1() -> PushRequest {
        PushRequest {
            updates: vec![("p".into(), vec![5])],
        }
    }

    #[test]
    fn v1_bytes_are_unchanged() {
        assert_eq!(
            SyncRequest::default().encode(),
            vec![1, 0, 0, 0, 0, 0, 0, 0]
        );
        let req = SyncRequest {
            full: true,
            base: Some((3, 4)),
            ..Default::default()
        };
        assert_eq!(req.encode(), vec![1, 1, 1, 3, 4, 0, 0, 0, 0, 0]);
        assert_eq!(push_v1().encode(), vec![1, 1, 1, b'p', 1, 5]);
        let pr = PushResponse {
            created: 1,
            changed: 2,
            conflicts: 3,
        };
        assert_eq!(pr.encode(), vec![1, 1, 2, 3]);
    }

    #[test]
    fn v2_envelope_bytes() {
        // Version 2, the complete v1 push (length 6), then the restores.
        let push = PushRequestV2 {
            request: push_v1(),
            restores: vec![restore("r", None, &[7, 8])],
        };
        assert_eq!(
            push.encode(),
            vec![2, 6, 1, 1, 1, b'p', 1, 5, 1, 1, b'r', 0, 2, 7, 8]
        );
        // A present but empty purge version is a presence byte of 1.
        let empty = PushRequestV2 {
            request: push_v1(),
            restores: vec![restore("r", Some(&[]), &[])],
        };
        assert_eq!(
            empty.encode(),
            vec![2, 6, 1, 1, 1, b'p', 1, 5, 1, 1, b'r', 1, 0, 0]
        );
    }

    #[test]
    fn v2_messages_round_trip() {
        let req = SyncRequestV2 {
            request: SyncRequest {
                full: true,
                base: Some((u64::MAX, 7)),
                versions: vec![("p".into(), vec![1, 2])],
                purged: vec!["gone".into()],
                hub_purged_known: 2,
                blobs: vec![],
                budget: 1 << 20,
            },
            purges: vec![
                purge("a", None),
                purge("b", Some(&[])),
                purge("c", Some(&[7, 8])),
            ],
            restores: vec![
                restore("a", None, &[]),
                restore("b", Some(&[]), &[1]),
                restore("c", Some(&[2]), &[3, 4]),
            ],
        };
        assert_eq!(SyncRequestV2::decode(&req.encode()).unwrap(), req);

        let resp = SyncResponseV2 {
            response: SyncResponse {
                epoch: 42,
                seq: 300,
                updates: vec![("p".into(), vec![0; 100])],
                more: true,
                ..Default::default()
            },
            purges: vec![purge("x", None)],
            restores: vec![restore("y", None, &[9])],
        };
        assert_eq!(SyncResponseV2::decode(&resp.encode()).unwrap(), resp);

        let push = PushRequestV2 {
            request: push_v1(),
            restores: vec![restore("r", Some(&[]), &[])],
        };
        assert_eq!(PushRequestV2::decode(&push.encode()).unwrap(), push);

        let bare = SyncRequestV2::default();
        assert_eq!(SyncRequestV2::decode(&bare.encode()).unwrap(), bare);
    }

    #[test]
    fn absent_and_empty_versions_differ() {
        let absent = PushRequestV2 {
            request: push_v1(),
            restores: vec![restore("r", None, &[])],
        };
        let empty = PushRequestV2 {
            request: push_v1(),
            restores: vec![restore("r", Some(&[]), &[])],
        };
        assert_ne!(absent.encode(), empty.encode());
        let back = PushRequestV2::decode(&absent.encode()).unwrap();
        assert_eq!(back.restores[0].purge_version, None);
        let back = PushRequestV2::decode(&empty.encode()).unwrap();
        assert_eq!(back.restores[0].purge_version, Some(vec![]));

        let req = SyncRequestV2 {
            purges: vec![purge("p", None)],
            ..Default::default()
        };
        let back = SyncRequestV2::decode(&req.encode()).unwrap();
        assert_eq!(back.purges[0].version, None);
    }

    #[test]
    fn v2_rejects_wrong_versions() {
        let v2 = PushRequestV2 {
            request: push_v1(),
            restores: vec![],
        }
        .encode();
        // v1 readers refuse a v2 envelope, and v2 readers refuse v1 bytes.
        assert!(PushRequest::decode(&v2).is_err());
        assert!(PushRequestV2::decode(&push_v1().encode()).is_err());
        let resp = SyncResponse::default().encode();
        assert!(SyncResponseV2::decode(&resp).is_err());
        // An unknown envelope version.
        let mut other = v2.clone();
        other[0] = PURGE_PROTOCOL + 1;
        assert!(PushRequestV2::decode(&other).is_err());
        // A nested envelope is not a v1 message.
        let mut w = W::v2();
        w.bytes(&v2);
        w.restores(&[]);
        assert!(PushRequestV2::decode(&w.0).is_err());
        assert!(PushRequestV2::decode(&[]).is_err());
    }

    #[test]
    fn v2_rejects_truncated_messages() {
        let req = SyncRequestV2 {
            request: SyncRequest {
                versions: vec![("p".into(), vec![1, 2, 3])],
                ..Default::default()
            },
            purges: vec![purge("a", Some(&[4]))],
            restores: vec![restore("b", None, &[5, 6])],
        }
        .encode();
        for n in 0..req.len() {
            assert!(SyncRequestV2::decode(&req[..n]).is_err(), "prefix {n}");
        }
        let push = PushRequestV2 {
            request: push_v1(),
            restores: vec![restore("r", Some(&[7]), &[8])],
        }
        .encode();
        for n in 0..push.len() {
            assert!(PushRequestV2::decode(&push[..n]).is_err(), "prefix {n}");
        }
    }

    #[test]
    fn v2_rejects_trailing_bytes() {
        let push = PushRequestV2 {
            request: push_v1(),
            restores: vec![restore("r", None, &[])],
        }
        .encode();
        let mut long = push.clone();
        long.push(0);
        assert!(PushRequestV2::decode(&long).is_err());
        // Trailing bytes inside the embedded v1 message are refused too.
        let mut inner = push_v1().encode();
        inner.push(0);
        let mut w = W::v2();
        w.bytes(&inner);
        w.restores(&[]);
        assert!(PushRequestV2::decode(&w.0).is_err());
    }

    #[test]
    fn v2_rejects_bad_presence_flags() {
        let mut w = W::v2();
        w.bytes(&push_v1().encode());
        w.var(1);
        w.str("r");
        w.0.push(2);
        w.bytes(&[]);
        assert!(PushRequestV2::decode(&w.0).is_err());
    }

    #[test]
    fn v2_record_counts_are_bounded() {
        // A count with no records behind it is refused before allocating.
        let mut w = W::v2();
        w.bytes(&push_v1().encode());
        w.var(1000);
        assert!(PushRequestV2::decode(&w.0).is_err());

        // Counts must cover complete records, not just one byte each.
        let mut w = W::v2();
        w.bytes(&push_v1().encode());
        w.var(10);
        w.0.extend_from_slice(&[0u8; 10]);
        assert!(PushRequestV2::decode(&w.0).is_err());

        // A long-lived vault can exceed 65,536 tombstones. The available
        // message bytes bound allocation, rather than an arbitrary count.
        let n = (1 << 16) + 1;
        let mut w = W::v2();
        w.bytes(&push_v1().encode());
        w.var(n as u64);
        w.0.extend_from_slice(&vec![0u8; 3 * n]);
        let got = PushRequestV2::decode(&w.0).unwrap();
        assert_eq!(got.restores.len(), n);
    }

    #[test]
    fn device_layout_bytes() {
        // Version 2, the complete v1 request (length 8), one purge "a" with no
        // version and device "p" (presence 1, length 1, byte p), no restores.
        let req = SyncRequestV2 {
            purges: vec![purge_from("a", None, Some("p"))],
            ..Default::default()
        };
        assert_eq!(
            req.encode_with_device(true),
            vec![2, 8, 1, 0, 0, 0, 0, 0, 0, 0, 1, 1, b'a', 0, 1, 1, b'p', 0]
        );
        // Without the capability the same record is the fieldless layout.
        assert_eq!(
            req.encode_with_device(false),
            vec![2, 8, 1, 0, 0, 0, 0, 0, 0, 0, 1, 1, b'a', 0, 0]
        );
        assert_eq!(req.encode(), req.encode_with_device(false));
        // A present but empty device id is a presence byte of 1 and length 0.
        let empty = SyncRequestV2 {
            purges: vec![purge_from("a", None, Some(""))],
            ..Default::default()
        };
        assert_eq!(
            empty.encode_with_device(true),
            vec![2, 8, 1, 0, 0, 0, 0, 0, 0, 0, 1, 1, b'a', 0, 1, 0, 0]
        );
    }

    #[test]
    fn device_records_round_trip() {
        let req = SyncRequestV2 {
            request: SyncRequest {
                full: true,
                ..Default::default()
            },
            purges: vec![
                purge_from("a", None, None),
                purge_from("b", Some(&[]), Some("")),
                purge_from("c", Some(&[7]), Some("laptop ÿ")),
            ],
            restores: vec![restore("r", None, &[1])],
        };
        let bytes = req.encode_with_device(true);
        assert_eq!(
            SyncRequestV2::decode_with_device(&bytes, true).unwrap(),
            req
        );

        let resp = SyncResponseV2 {
            response: SyncResponse {
                epoch: 9,
                ..Default::default()
            },
            purges: req.purges.clone(),
            restores: vec![restore("y", Some(&[2]), &[3])],
        };
        assert_eq!(
            SyncResponseV2::decode_with_device(&resp.encode_with_device(true), true).unwrap(),
            resp
        );
    }

    #[test]
    fn fieldless_and_device_layouts_refuse_each_other() {
        let with = SyncResponseV2 {
            purges: vec![purge_from("x", Some(&[7, 8]), Some("laptop"))],
            ..Default::default()
        };
        // The fieldless answer drops the device, and reads back as it was
        // before the capability.
        let plain = with.encode();
        let back = SyncResponseV2::decode(&plain).unwrap();
        assert_eq!(back.purges, vec![purge("x", Some(&[7, 8]))]);
        // Read as device-carrying, the trailing count is taken for a device
        // flag and the message ends early: refused, not misread.
        assert!(SyncResponseV2::decode_with_device(&plain, true).is_err());
        // Device-carrying bytes read fieldless leave bytes over: refused.
        assert!(SyncResponseV2::decode(&with.encode_with_device(true)).is_err());
    }

    #[test]
    fn malformed_device_fields_are_errors() {
        // A device presence byte other than 0 or 1.
        let mut w = W::v2();
        w.bytes(&SyncRequest::default().encode());
        w.var(1);
        w.str("a");
        w.0.push(0);
        w.0.push(2);
        w.var(0);
        assert!(SyncRequestV2::decode_with_device(&w.0, true).is_err());

        // A present device id that is not UTF-8.
        let mut w = W::v2();
        w.bytes(&SyncRequest::default().encode());
        w.var(1);
        w.str("a");
        w.0.push(0);
        w.0.push(1);
        w.bytes(&[0xff]);
        w.var(0);
        assert!(SyncRequestV2::decode_with_device(&w.0, true).is_err());

        // Every truncation of a device-carrying message is refused.
        let req = SyncRequestV2 {
            purges: vec![purge_from("a", Some(&[4]), Some("p"))],
            restores: vec![restore("b", None, &[5])],
            ..Default::default()
        }
        .encode_with_device(true);
        for n in 0..req.len() {
            assert!(
                SyncRequestV2::decode_with_device(&req[..n], true).is_err(),
                "prefix {n}"
            );
        }
    }

    #[test]
    fn device_record_counts_are_bounded() {
        // Two device-carrying records need at least six bytes; five cannot
        // hold them, so the count is refused before allocating.
        let mut w = W::v2();
        w.bytes(&SyncRequest::default().encode());
        w.var(2);
        w.0.extend_from_slice(&[0u8; 5]);
        assert!(SyncRequestV2::decode_with_device(&w.0, true).is_err());
    }
}
