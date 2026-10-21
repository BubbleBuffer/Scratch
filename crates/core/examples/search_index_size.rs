//! Reproducible storage-option probe; run on a quiet linux01 in release mode.
//! cargo run --release -p vault_core --example search_index_size -- /tmp/notes-index-size
//! Keeps the seeded vault and each option database for dbstat inspection.
use std::{fs, path::Path, time::Instant};

use rusqlite::Connection;
use vault_core::{seed, Vault};

const QUERIES: &[&str] = &[
    "p*",
    "pr*",
    "pro*",
    "proj*",
    "project*",
    "g*",
    "ga*",
    "gar*",
    "gard*",
    "garden*",
    "a*",
    "t*",
    "tomato*",
    "weekly AND review*",
    "coffee AND mor*",
    "zür*",
    "café*",
    "big AND page*",
    "xylophone*",
    "\"weekly review\"",
    "NEAR(coffee morning, 5)",
    "title:project*",
];

const API_QUERIES: &[&str] = &[
    "p",
    "pr",
    "pro",
    "proj",
    "project",
    "g",
    "ga",
    "gar",
    "gard",
    "garden",
    "a",
    "t",
    "tomato",
    "weekly review",
    "coffee mor",
    "zür",
    "café",
    "big page",
    "xylophone",
    "tomatoe",
    "gardne",
    "weekyl review",
    "cofee",
    "cofee mor",
    "projcet",
    "recipies",
    "gadr",
    "meeitng",
    "buget",
    "desgin",
    "journla",
    "databsae",
    "\"weekly review\"",
    "garden tag:garden",
    "-tag:work",
    "title:garden",
    "type:document",
];

fn answer(vault: &Vault, query: &str) -> String {
    let spec = vault.parse_search(query.into()).unwrap().spec;
    serde_json::to_string(&(
        vault.search_adaptive(query.into(), 50).unwrap(),
        vault.search_full(query.into(), 50).unwrap(),
        vault.search_pages(query.into(), 50).unwrap(),
        vault.search_spec(spec, 50, 0).unwrap(),
    ))
    .unwrap()
}

fn api_times(reference: &Vault, candidate: &Vault, label: &str) {
    for query in API_QUERIES {
        for _ in 0..3 {
            reference.search((*query).into(), 50).unwrap();
            candidate.search((*query).into(), 50).unwrap();
        }
        let mut old = Vec::new();
        let mut new = Vec::new();
        for sample in 0..50 {
            let order = if sample % 2 == 0 {
                [(reference, &mut old), (candidate, &mut new)]
            } else {
                [(candidate, &mut new), (reference, &mut old)]
            };
            for (vault, samples) in order {
                let start = Instant::now();
                let _ = vault.search((*query).into(), 50).unwrap();
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        old.sort_by(f64::total_cmp);
        new.sort_by(f64::total_cmp);
        println!(
            "API_PAIR {label} {query:?} old_p50={:.3} new_p50={:.3} old_p95={:.3} new_p95={:.3} n=50",
            old[24], new[24], old[47], new[47]
        );
    }
}

type Hit = (i64, f64, Option<String>, Option<String>);
fn hits(conn: &Connection, query: &str) -> rusqlite::Result<Vec<Hit>> {
    conn.prepare_cached(
        "SELECT rowid, bm25(fts,10.0,1.0), snippet(fts,1,'','','…',12),
                highlight(fts,1,'[',']') FROM fts WHERE fts MATCH ?1
         ORDER BY bm25(fts,10.0,1.0), rowid LIMIT 50",
    )?
    .query_map([query], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })?
    .collect()
}

fn report(conn: &Connection, path: &Path, label: &str) {
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    println!(
        "SIZE {label} total={} page_size={} freelist={}",
        fs::metadata(path).unwrap().len(),
        conn.query_row("PRAGMA page_size", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        conn.query_row("PRAGMA freelist_count", [], |r| r.get::<_, i64>(0))
            .unwrap()
    );
    let mut stmt = conn
        .prepare("SELECT name, sum(pgsize) FROM dbstat GROUP BY name ORDER BY name")
        .unwrap();
    for row in stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .unwrap()
    {
        let (name, bytes) = row.unwrap();
        println!("TABLE {label} {name} {bytes}");
    }
}

fn main() {
    assert!(!cfg!(debug_assertions), "use --release");
    let size_only = std::env::args().any(|a| a == "--size-only");
    let reuse = std::env::args().any(|a| a == "--reuse");
    let only = std::env::args().find_map(|a| a.strip_prefix("--only=").map(str::to_owned));
    let root = std::env::args().nth(1).expect("vault directory");
    if !Path::new(&root).join(".vault/index.sqlite").exists() {
        seed::generate(root.clone(), 9_999, 2_000).unwrap();
    }
    let vault = Vault::open(root.clone()).unwrap();
    assert_eq!(vault.page_count().unwrap(), 10_000);
    vault.flush().unwrap();
    let expected_api: Vec<_> = API_QUERIES.iter().map(|q| answer(&vault, q)).collect();
    drop(vault);
    let baseline = Path::new(&root).join(".vault/index.sqlite");
    let conn = Connection::open(&baseline).unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    report(&conn, &baseline, "before");
    let expected: Vec<_> = QUERIES.iter().map(|q| hits(&conn, q).unwrap()).collect();
    drop(conn);
    let reference = (!size_only).then(|| Vault::open(root.clone()).unwrap());
    let options = [
        ("rebuilt123", "prefix='1 2 3'", "", ""),
        (
            "external123",
            "prefix='1 2 3', content='probe_text'",
            "",
            "",
        ),
        (
            "external_view123",
            "prefix='1 2 3', content='probe_text'",
            "",
            "",
        ),
        ("contentless123", "prefix='1 2 3', content=''", "", ""),
        ("detail_column", "prefix='1 2 3', detail=column", "", ""),
        ("detail_none", "prefix='1 2 3', detail=none", "", ""),
        ("columnsize0", "prefix='1 2 3', columnsize=0", "", ""),
        ("prefix23", "prefix='2 3'", "", ""),
        ("prefix12", "prefix='1 2'", "", ""),
        ("prefix1", "prefix='1'", "", ""),
        ("prefix2", "prefix='2'", "", ""),
        ("prefix3", "prefix='3'", "", ""),
        ("prefix_none", "detail=full", "", ""),
        (
            "prefix_none_page8192",
            "detail=full",
            "PRAGMA page_size=8192; VACUUM;",
            "",
        ),
        (
            "prefix1_page8192",
            "prefix='1'",
            "PRAGMA page_size=8192; VACUUM;",
            "",
        ),
        (
            "prefix12_page8192",
            "prefix='1 2'",
            "PRAGMA page_size=8192; VACUUM;",
            "",
        ),
        (
            "page1024",
            "prefix='1 2 3'",
            "PRAGMA page_size=1024; VACUUM;",
            "",
        ),
        (
            "page8192",
            "prefix='1 2 3'",
            "PRAGMA page_size=8192; VACUUM;",
            "",
        ),
        (
            "page16384",
            "prefix='1 2 3'",
            "PRAGMA page_size=16384; VACUUM;",
            "",
        ),
        (
            "auto_full",
            "prefix='1 2 3'",
            "PRAGMA auto_vacuum=FULL; VACUUM;",
            "",
        ),
        (
            "auto_incremental",
            "prefix='1 2 3'",
            "PRAGMA auto_vacuum=INCREMENTAL; VACUUM;",
            "",
        ),
        ("vacuum_only", "", "", "VACUUM;"),
        (
            "optimize",
            "",
            "",
            "INSERT INTO fts(fts) VALUES('optimize');",
        ),
        (
            "optimize_vacuum",
            "",
            "",
            "INSERT INTO fts(fts) VALUES('optimize'); VACUUM;",
        ),
    ];
    for (label, config, before, after) in options {
        if only
            .as_ref()
            .is_some_and(|list| !list.split(',').any(|item| item == label))
        {
            continue;
        }
        let path = Path::new(&root).join(format!("{label}.sqlite"));
        let existing = path.exists();
        assert!(
            !existing || reuse,
            "option database already exists: {}",
            path.display()
        );
        if !existing {
            fs::copy(&baseline, &path).unwrap();
        }
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA cache_size=-16000; PRAGMA temp_store=MEMORY;",
        )
        .unwrap();
        if !existing {
            conn.execute_batch(before).unwrap();
        }
        if !existing && !config.is_empty() {
            conn.execute_batch(
                "CREATE TEMP TABLE probe_copy AS SELECT rowid,title,body FROM fts; DROP TABLE fts;",
            )
            .unwrap();
            if label == "external123" {
                conn.execute_batch("CREATE TABLE probe_text(rowid INTEGER PRIMARY KEY,title TEXT,body TEXT); INSERT INTO probe_text SELECT * FROM probe_copy;").unwrap();
            }
            if label == "external_view123" {
                conn.execute_batch(
                    "CREATE TABLE probe_body(rowid INTEGER PRIMARY KEY,body TEXT);
                     INSERT INTO probe_body SELECT rowid,body FROM probe_copy WHERE rowid>0;
                     CREATE VIEW probe_text AS
                       SELECT p.rowid,p.title,b.body FROM pages p JOIN probe_body b ON b.rowid=p.rowid
                       UNION ALL SELECT -rowid,title,body FROM search_external;"
                ).unwrap();
            }
            conn.execute_batch(&format!(
                "CREATE VIRTUAL TABLE fts USING fts5(title,body,tokenize='unicode61 remove_diacritics 2',{config});
                 INSERT INTO fts(rowid,title,body) SELECT rowid,title,body FROM probe_copy; DROP TABLE probe_copy;"
            )).unwrap();
        }
        if !existing {
            conn.execute_batch(after).unwrap();
        }
        // Report actual file size before compaction too: a table rebuild can
        // leave the original database's free pages behind.
        report(&conn, &path, &format!("{label}_raw"));
        conn.execute_batch("VACUUM").unwrap();
        report(&conn, &path, label);
        let mut compatible = true;
        for (query, reference) in QUERIES.iter().zip(&expected) {
            match hits(&conn, query) {
                Ok(actual) if &actual == reference => {}
                Ok(_) => {
                    println!(
                        "INCOMPATIBLE {label} {query:?}: rank/snippet/highlight/results changed"
                    );
                    compatible = false;
                    break;
                }
                Err(error) => {
                    println!("INCOMPATIBLE {label} {query:?}: {error}");
                    compatible = false;
                    break;
                }
            }
        }
        if !compatible {
            continue;
        }
        println!("COMPATIBLE {label}");
        drop(conn);
        let api_root = Path::new(&root).join(format!("{label}-vault"));
        fs::create_dir_all(api_root.join(".vault")).unwrap();
        fs::copy(&path, api_root.join(".vault/index.sqlite")).unwrap();
        let vault = Vault::open(api_root.to_string_lossy().into_owned()).unwrap();
        for (query, expected) in API_QUERIES.iter().zip(&expected_api) {
            assert_eq!(
                &answer(&vault, query),
                expected,
                "API parity: {label} {query:?}"
            );
        }
        println!("API_COMPATIBLE {label}");
        if size_only {
            continue;
        }
        api_times(reference.as_ref().unwrap(), &vault, label);
        drop(vault);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA cache_size=-16000; PRAGMA temp_store=MEMORY;")
            .unwrap();
        for query in QUERIES {
            for _ in 0..3 {
                hits(&conn, query).unwrap();
            }
            let mut samples = Vec::new();
            for _ in 0..30 {
                let start = Instant::now();
                let _ = hits(&conn, query).unwrap();
                samples.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "SQL_TIME {label} {query:?} p50={:.3} p95={:.3}",
                samples[14], samples[28]
            );
        }
    }
}
