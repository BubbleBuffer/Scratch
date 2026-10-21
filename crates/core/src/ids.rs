//! ULID-style ids (48-bit millisecond time + 80 random bits, Crockford
//! base32) without a random-number dependency: randomness comes from std's
//! randomly keyed SipHash, mixed with a counter, the time and the thread.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

struct ClockState {
    next: Option<i64>,
    last_stamp: i64,
}

static CLOCK: Mutex<ClockState> = Mutex::new(ClockState {
    next: None,
    last_stamp: 0,
});

/// Sets a process-wide deterministic clock, or restores the system clock.
/// Call only with no open vaults. Stamps remain unique across clock resets.
/// Each subsequent clock read advances by one millisecond, across threads.
pub fn set_test_clock(start_ms: Option<i64>) {
    let mut clock = CLOCK.lock().unwrap_or_else(|e| e.into_inner());
    clock.next = start_ms;
}

// Keep the test hook reachable through the existing public Vault surface;
// the ids module itself is intentionally private.
impl crate::Vault {
    pub fn set_test_clock(start_ms: Option<i64>) {
        set_test_clock(start_ms);
    }
}

fn read_clock(clock: &mut ClockState) -> i64 {
    if let Some(next) = clock.next {
        clock.next = Some(next.checked_add(1).expect("test clock exhausted"));
        return next;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn now_ms() -> i64 {
    read_clock(&mut CLOCK.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The time to record for a page created, edited or trashed here: the
/// current time in milliseconds, but always after the last stamp handed
/// out in this process, so pages made or edited one after another keep
/// that order in "last edited" lists even within one millisecond.
pub(crate) fn stamp_ms() -> i64 {
    let mut clock = CLOCK.lock().unwrap_or_else(|e| e.into_inner());
    let now = read_clock(&mut clock);
    clock.last_stamp = now.max(clock.last_stamp + 1);
    clock.last_stamp
}

fn random_u64() -> u64 {
    static STATE: OnceLock<RandomState> = OnceLock::new();
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = STATE.get_or_init(RandomState::new).build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    std::hash::Hash::hash(&std::thread::current().id(), &mut h);
    h.finish()
}

/// A new 26-character id, sortable by creation time.
pub fn new_id() -> String {
    let time = (now_ms() as u128) & ((1u128 << 48) - 1);
    let rand =
        ((random_u64() as u128) << 16 | (random_u64() as u128 & 0xffff)) & ((1u128 << 80) - 1);
    encode_id((time << 80) | rand)
}

/// A day's identity belongs to its actual Journal, including a legacy one.
/// The caller validates the ISO calendar day before deriving this key.
pub(crate) fn journal_day_id(journal: &str, iso_day: &str) -> String {
    let digest = crate::attach::sha256_hex(
        format!("notes-app:journal-day:v1\0{journal}\0{iso_day}").as_bytes(),
    );
    encode_id(u128::from_str_radix(&digest[..32], 16).expect("SHA-256 hex"))
}

/// The page id of one occurrence of a schedule. `utc` is the occurrence's
/// canonical RFC 3339 UTC instant. Retries and a second runner derive the same
/// id, so they reach one page.
pub fn schedule_occurrence_id(schedule: &str, utc: &str) -> String {
    let digest = crate::attach::sha256_hex(
        format!("notes-app:schedule-occurrence:v1\0{schedule}\0{utc}").as_bytes(),
    );
    encode_id(u128::from_str_radix(&digest[..32], 16).expect("SHA-256 hex"))
}

/// The deterministic page id of a device's node record.
pub fn node_record_id(device: &str) -> String {
    let digest =
        crate::attach::sha256_hex(format!("notes-app:node-record:v1\0{device}").as_bytes());
    encode_id(u128::from_str_radix(&digest[..32], 16).expect("SHA-256 hex"))
}

/// Whether `id` has the shape of a page id: 26 characters of the id alphabet,
/// with a leading character in `0`..=`7`. The 26 characters carry 130 bits,
/// and a 128-bit value's leading character is at most `7`. Every generated id
/// (`new_id`, `schedule_occurrence_id`, `node_record_id`) is a 128-bit value,
/// so all of them pass. Ids become file names, so anything else is refused.
pub fn is_page_id(id: &str) -> bool {
    id.len() == 26
        && id.as_bytes()[0] <= b'7'
        && id.bytes().all(|b| ALPHABET.contains(&b))
}

fn encode_id(mut v: u128) -> String {
    let mut out = [0u8; 26];
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[(v & 31) as usize];
        v >>= 5;
    }
    String::from_utf8(out.to_vec()).expect("ascii")
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_clock() {
        // Isolate the process-wide hook from the other parallel core tests.
        const CHILD: &str = "NOTES_CLOCK_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "ids::tests::test_clock"])
                .env(CHILD, "1")
                .status()
                .unwrap();
            assert!(result.success());
            return;
        }
        let start = 1_791_278_400_000;
        super::set_test_clock(Some(start));
        assert_eq!(super::now_ms(), start);
        assert_eq!(super::stamp_ms(), start + 1);
        assert_eq!(super::stamp_ms(), start + 2);
        let threads: Vec<_> = (0..4)
            .map(|_| std::thread::spawn(|| (0..1000).map(|_| super::now_ms()).collect::<Vec<_>>()))
            .collect();
        let mut times: Vec<_> = threads
            .into_iter()
            .flat_map(|t| t.join().unwrap())
            .collect();
        times.sort_unstable();
        assert_eq!(times, (start + 3..start + 4003).collect::<Vec<_>>());
        let ids: std::collections::HashSet<_> = (0..1000).map(|_| super::new_id()).collect();
        assert_eq!(ids.len(), 1000);
        super::set_test_clock(Some(start));
        assert_eq!(super::now_ms(), start);
        assert_eq!(super::stamp_ms(), start + 3);
        super::set_test_clock(None);
        let real = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!((super::now_ms() - real).abs() < 20_000);
    }

    #[test]
    fn ids_are_unique_and_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..10_000 {
            let id = super::new_id();
            assert_eq!(id.len(), 26);
            assert!(seen.insert(id));
        }
    }

    #[test]
    fn stamps_strictly_increase() {
        let mut last = super::stamp_ms();
        for _ in 0..10_000 {
            let t = super::stamp_ms();
            assert!(t > last);
            last = t;
        }
        // Close to the real time (10,000 stamps run ahead by at most 10 s).
        assert!((last - super::now_ms()).abs() < 20_000);
    }
}
