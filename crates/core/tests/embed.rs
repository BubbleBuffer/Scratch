//! The generic `embed` block: it stores its target id only, and its title is
//! looked up when the block is searched, exported or drawn.
use std::{fs, path::PathBuf};
use vault_core::*;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("notes_embed_{}", new_id())))
    }
    fn open(&self) -> Vault {
        Vault::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn embed_op(id: &str, text: &str, target: Option<&str>) -> Vec<BlockOp> {
    vec![
        BlockOp::Insert {
            parent: None,
            after: None,
            block: NewBlock {
                id: id.into(),
                kind: BlockKind::Embed,
                text: text.into(),
                marks: vec![],
                checked: false,
            },
        },
        BlockOp::SetTarget {
            id: id.into(),
            target: target.map(Into::into),
        },
    ]
}

fn hit_ids(v: &Vault, query: &str) -> Vec<PageId> {
    v.search(query.into(), 20)
        .unwrap()
        .into_iter()
        .map(|h| h.id)
        .collect()
}

#[test]
fn embed_stores_target_id_only_and_reopens_with_its_title_searchable() {
    let t = Temp::new();
    let (host, target) = {
        let v = t.open();
        let target = v.create_page(None, "Lisbon".into()).unwrap();
        let host = v.create_page(None, "Trip".into()).unwrap();
        // The text is a stale title: it must not be kept.
        v.apply(
            host.clone(),
            embed_op("e", "Stale title", Some(target.as_str())),
        )
        .unwrap();
        v.flush().unwrap();
        (host, target)
    };
    let v = t.open();
    let loaded = v.load_page(host.clone()).unwrap();
    let block = &loaded.blocks[0];
    assert_eq!(block.kind, BlockKind::Embed);
    assert_eq!(block.target.as_deref(), Some(target.as_str()));
    assert_eq!(block.text, "");
    assert!(block.marks.is_empty());

    // Search finds the host by the target's title, never by the target id.
    assert!(hit_ids(&v, "Lisbon").contains(&host));
    assert!(!hit_ids(&v, &target).contains(&host));
    assert!(!hit_ids(&v, "Stale").contains(&host));
}

#[test]
fn embed_text_is_dropped_when_set_or_when_the_kind_changes() {
    let t = Temp::new();
    let v = t.open();
    let target = v.create_page(None, "Porto".into()).unwrap();
    let page = v.create_page(None, "Host".into()).unwrap();
    v.apply(page.clone(), embed_op("e", "Porto", Some(target.as_str())))
        .unwrap();
    v.apply(
        page.clone(),
        vec![BlockOp::SetText {
            id: "e".into(),
            text: "Written over".into(),
            marks: vec![],
        }],
    )
    .unwrap();
    v.apply(
        page.clone(),
        vec![
            BlockOp::Insert {
                parent: None,
                after: Some("e".into()),
                block: NewBlock {
                    id: "p".into(),
                    kind: BlockKind::Paragraph,
                    text: "Porto".into(),
                    marks: vec![],
                    checked: false,
                },
            },
            BlockOp::SetKind {
                id: "p".into(),
                kind: BlockKind::Embed,
            },
            BlockOp::SetTarget {
                id: "p".into(),
                target: Some(target.clone()),
            },
        ],
    )
    .unwrap();
    v.flush().unwrap();
    drop(v);
    let v = t.open();
    let loaded = v.load_page(page).unwrap();
    assert_eq!(loaded.blocks.len(), 2);
    for block in &loaded.blocks {
        assert_eq!(block.kind, BlockKind::Embed);
        assert_eq!(block.target.as_deref(), Some(target.as_str()));
        assert_eq!(block.text, "");
    }
}

#[test]
fn embed_markdown_shows_the_live_title_and_reimports_the_target_id() {
    let t = Temp::new();
    let v = t.open();
    let target = v.create_page(None, "Lisbon".into()).unwrap();
    let host = v.create_page(None, "Trip".into()).unwrap();
    v.apply(host.clone(), embed_op("e", "", Some(target.as_str())))
        .unwrap();

    let md = v.page_markdown(host.clone()).unwrap();
    assert!(md.starts_with("> Lisbon <!-- block="), "{md}");

    // A rename shows in the next export, with the same target id.
    v.rename_page(target.clone(), "Porto".into()).unwrap();
    let md = v.page_markdown(host.clone()).unwrap();
    assert!(md.starts_with("> Porto <!-- block="), "{md}");
    assert!(!md.contains("Lisbon"), "{md}");

    // Edited fallback text is ignored: the id decides which note it is.
    let edited = md.replacen("> Porto <!--", "> Anything else <!--", 1);
    let parsed = v.parse_markdown(edited);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].kind, BlockKind::Embed);
    assert_eq!(parsed[0].target.as_deref(), Some(target.as_str()));
    assert_eq!(parsed[0].text, "");
}

#[test]
fn embed_to_unknown_target_keeps_the_id_through_reopen_export_and_import() {
    let (source_dir, destination_dir) = (Temp::new(), Temp::new());
    let missing = "page-not-in-this-vault";
    let host = {
        let source = source_dir.open();
        let host = source.create_page(None, "Orphan".into()).unwrap();
        source
            .apply(host.clone(), embed_op("e", "", Some(missing)))
            .unwrap();
        source.flush().unwrap();
        host
    };
    let source = source_dir.open();
    let loaded = source.load_page(host.clone()).unwrap();
    assert_eq!(loaded.blocks[0].kind, BlockKind::Embed);
    assert_eq!(loaded.blocks[0].target.as_deref(), Some(missing));
    assert_eq!(loaded.blocks[0].text, "");

    // No title is known, so the fallback is empty; the id stays in the comment.
    let md = source.page_markdown(host.clone()).unwrap();
    assert!(md.starts_with("> <!-- block="), "{md}");
    assert!(!md.contains(missing), "the id is hex-encoded: {md}");
    let parsed = source.parse_markdown(md);
    assert_eq!(parsed[0].target.as_deref(), Some(missing));
    assert_eq!(parsed[0].text, "");

    // Through a markdown file into another vault.
    source.flush().unwrap();
    let destination = destination_dir.open();
    let report = destination
        .import_markdown(
            source_dir.0.join("pages").to_string_lossy().into_owned(),
            None,
        )
        .unwrap();
    assert_eq!(report.pages, 1);
    let imported = destination.load_page(report.top_pages[0].clone()).unwrap();
    assert_eq!(imported.blocks[0].kind, BlockKind::Embed);
    assert_eq!(imported.blocks[0].target.as_deref(), Some(missing));
    assert_eq!(imported.blocks[0].text, "");
}

#[test]
fn reindex_gives_embeds_the_titles_of_notes_indexed_later() {
    let t = Temp::new();
    let hosts = {
        let v = t.open();
        let mut hosts = Vec::new();
        // Several hosts, each embedding a note made after it.
        for n in 0..8 {
            let host = v.create_page(None, format!("Host {n}")).unwrap();
            let target = v.create_page(None, format!("Quasarville {n}")).unwrap();
            v.apply(host.clone(), embed_op("e", "", Some(target.as_str())))
                .unwrap();
            hosts.push((host, n));
        }
        v.flush().unwrap();
        hosts
    };
    let v = t.open();
    v.reindex().unwrap();
    for (host, n) in hosts {
        let ids = hit_ids(&v, &format!("Quasarville {n}"));
        assert!(
            ids.contains(&host),
            "host {n} not found by its target title"
        );
    }
}

fn chat_row(id: &str, title: &str) -> ExternalSearchRow {
    ExternalSearchRow {
        id: id.into(),
        title: title.into(),
        body: "Outline summary".into(),
        page_type: "chat".into(),
        target: Some("opaque navigation target".into()),
        updated_ms: 10,
    }
}

fn page_file(dir: &Temp, id: &str) -> String {
    let pages = dir.0.join("pages");
    let name = fs::read_dir(&pages)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .find(|n| n.contains(id))
        .expect("page file");
    fs::read_to_string(pages.join(name)).unwrap()
}

#[test]
fn target_rename_after_flush_updates_the_host_search_text_and_markdown() {
    let t = Temp::new();
    let v = t.open();
    let target = v.create_page(None, "Lisbon".into()).unwrap();
    let host = v.create_page(None, "Trip".into()).unwrap();
    v.apply(host.clone(), embed_op("e", "", Some(target.as_str())))
        .unwrap();
    v.flush().unwrap();
    assert!(hit_ids(&v, "Lisbon").contains(&host));

    v.rename_page(target.clone(), "Porto".into()).unwrap();
    v.flush().unwrap();
    assert!(hit_ids(&v, "Porto").contains(&host));
    assert!(!hit_ids(&v, "Lisbon").contains(&host));
    let file = page_file(&t, &host);
    assert!(file.contains("> Porto <!-- block="), "{file}");
    assert!(!file.contains("Lisbon"), "{file}");
}

#[test]
fn external_row_changes_update_the_embedding_page_after_it_is_saved() {
    let t = Temp::new();
    let v = t.open();
    let host = v.create_page(None, "Trip".into()).unwrap();
    v.apply(host.clone(), embed_op("e", "", Some("chat-1")))
        .unwrap();
    v.flush().unwrap();
    assert!(!hit_ids(&v, "Alpha").contains(&host));

    v.upsert_search_row("agent".into(), chat_row("chat-1", "Alpha plan"))
        .unwrap();
    v.flush().unwrap();
    assert!(hit_ids(&v, "Alpha").contains(&host));

    v.upsert_search_row("agent".into(), chat_row("chat-1", "Beta plan"))
        .unwrap();
    v.flush().unwrap();
    assert!(!hit_ids(&v, "Alpha").contains(&host));
    assert!(hit_ids(&v, "Beta").contains(&host));

    v.delete_search_row("agent".into(), "chat-1".into())
        .unwrap();
    v.flush().unwrap();
    assert!(!hit_ids(&v, "Beta").contains(&host));
    // The stored embed keeps its target id throughout.
    let block = &v.load_page(host).unwrap().blocks[0];
    assert_eq!(block.target.as_deref(), Some("chat-1"));
    assert_eq!(block.text, "");
}

#[test]
fn clearing_an_external_source_empties_its_embeds_and_leaves_other_sources() {
    let t = Temp::new();
    let v = t.open();
    let (plans, notes, other) = (
        v.create_page(None, "Plans".into()).unwrap(),
        v.create_page(None, "Notes".into()).unwrap(),
        v.create_page(None, "Other".into()).unwrap(),
    );
    v.apply(plans.clone(), embed_op("e", "", Some("chat-1")))
        .unwrap();
    v.apply(notes.clone(), embed_op("e", "", Some("chat-2")))
        .unwrap();
    v.apply(other.clone(), embed_op("e", "", Some("chat-3")))
        .unwrap();
    v.upsert_search_row("agent".into(), chat_row("chat-1", "Alpha plan"))
        .unwrap();
    v.upsert_search_row("agent".into(), chat_row("chat-2", "Gamma plan"))
        .unwrap();
    v.upsert_search_row("kept".into(), chat_row("chat-3", "Delta plan"))
        .unwrap();
    v.flush().unwrap();
    assert!(hit_ids(&v, "Alpha").contains(&plans));
    assert!(hit_ids(&v, "Gamma").contains(&notes));
    assert!(hit_ids(&v, "Delta").contains(&other));

    v.clear_search_source("agent".into()).unwrap();
    v.flush().unwrap();
    assert!(!hit_ids(&v, "Alpha").contains(&plans));
    assert!(!hit_ids(&v, "Gamma").contains(&notes));
    assert!(hit_ids(&v, "Delta").contains(&other));
}

#[test]
fn an_ambiguous_external_id_leaves_embeds_empty_until_one_row_remains() {
    let t = Temp::new();
    let v = t.open();
    let host = v.create_page(None, "Trip".into()).unwrap();
    v.apply(host.clone(), embed_op("e", "", Some("chat-1")))
        .unwrap();
    v.upsert_search_row("agent".into(), chat_row("chat-1", "Alpha plan"))
        .unwrap();
    v.flush().unwrap();
    assert!(hit_ids(&v, "Alpha").contains(&host));

    // A second row with the same id makes the target ambiguous: no title.
    v.upsert_search_row("other".into(), chat_row("chat-1", "Beta plan"))
        .unwrap();
    v.flush().unwrap();
    assert!(!hit_ids(&v, "Alpha").contains(&host));
    assert!(!hit_ids(&v, "Beta").contains(&host));

    v.delete_search_row("other".into(), "chat-1".into())
        .unwrap();
    v.flush().unwrap();
    assert!(hit_ids(&v, "Alpha").contains(&host));
}

#[test]
fn importing_pages_mints_new_ids_so_an_embed_of_the_old_target_stays_empty() {
    // Known limit, pinned: the importer gives every page a new id and does
    // not remap an embed's stored target id, so the embed of the old id stays
    // unknown (empty) even after the target is imported, and after a reindex.
    // A later importer fix should change this test.
    let (source_dir, pages_dir, destination_dir) = (Temp::new(), Temp::new(), Temp::new());
    let target = {
        let source = source_dir.open();
        let target = source.create_page(None, "Lisbon".into()).unwrap();
        let host = source.create_page(None, "Trip".into()).unwrap();
        source
            .apply(host.clone(), embed_op("e", "", Some(target.as_str())))
            .unwrap();
        source.flush().unwrap();
        target
    };
    let files = source_dir.0.join("pages");
    for e in fs::read_dir(&files).unwrap() {
        let p = e.unwrap().path();
        let title_dir = if p.to_string_lossy().contains(target.as_str()) {
            "target"
        } else {
            "host"
        };
        fs::create_dir_all(pages_dir.0.join(title_dir)).unwrap();
        fs::copy(&p, pages_dir.0.join(title_dir).join(p.file_name().unwrap())).unwrap();
    }
    let destination = destination_dir.open();
    destination
        .import_markdown(
            pages_dir.0.join("host").to_string_lossy().into_owned(),
            None,
        )
        .unwrap();
    destination.flush().unwrap();
    destination
        .import_markdown(
            pages_dir.0.join("target").to_string_lossy().into_owned(),
            None,
        )
        .unwrap();
    destination.flush().unwrap();

    let host_new = destination.search("Trip".into(), 20).unwrap()[0].id.clone();
    let lisbon_new = destination.search("Lisbon".into(), 20).unwrap()[0]
        .id
        .clone();
    assert_ne!(lisbon_new, target, "importer mints new page ids");
    let stored = destination.load_page(host_new.clone()).unwrap().blocks;
    let embed = stored.iter().find(|b| b.kind == BlockKind::Embed).unwrap();
    assert_eq!(embed.target.as_deref(), Some(target.as_str()));

    let hits = |v: &Vault| -> Vec<String> {
        v.search("Lisbon".into(), 20)
            .unwrap()
            .into_iter()
            .map(|h| h.id)
            .collect()
    };
    assert!(!hits(&destination).contains(&host_new));
    destination.reindex().unwrap();
    destination.flush().unwrap();
    assert!(!hits(&destination).contains(&host_new));
}

#[test]
fn single_folder_import_points_a_known_embed_at_the_new_target() {
    // "Alpha trip" sorts before "Zulu city", so the host is imported first.
    let (source_dir, destination_dir) = (Temp::new(), Temp::new());
    let missing = "page-not-in-this-vault";
    let (host, target, mid) = {
        let source = source_dir.open();
        let target = source.create_page(None, "Zulu city".into()).unwrap();
        let host = source.create_page(None, "Alpha trip".into()).unwrap();
        let mid = source.create_page(None, "Mid notes".into()).unwrap();
        source
            .apply(host.clone(), embed_op("e", "", Some(target.as_str())))
            .unwrap();
        source
            .apply(mid.clone(), embed_op("e", "", Some(missing)))
            .unwrap();
        source.flush().unwrap();
        (host, target, mid)
    };
    let destination = destination_dir.open();
    let folder = source_dir.0.join("pages").to_string_lossy().into_owned();
    destination.import_markdown(folder, None).unwrap();
    destination.flush().unwrap();

    let host_new = destination.search("Alpha trip".into(), 20).unwrap()[0]
        .id
        .clone();
    assert_ne!(host_new, host);
    let loaded = destination.load_page(host_new.clone()).unwrap();
    assert_eq!(loaded.blocks[0].kind, BlockKind::Embed);
    let target_new = loaded.blocks[0].target.clone().unwrap();
    assert_ne!(target_new, target, "the embed follows the imported target");
    assert_eq!(
        destination.load_page(target_new.clone()).unwrap().title,
        "Zulu city"
    );
    assert_eq!(loaded.blocks[0].text, "");
    // The search body was rerendered after the whole import.
    assert!(hit_ids(&destination, "Zulu").contains(&host_new));

    // A target outside the folder keeps its id; the mid page is not touched.
    let mid_new = destination.search("Mid notes".into(), 20).unwrap()[0]
        .id
        .clone();
    assert_ne!(mid_new, mid);
    let mid_loaded = destination.load_page(mid_new).unwrap();
    assert_eq!(mid_loaded.blocks[0].target.as_deref(), Some(missing));
    assert_eq!(mid_loaded.blocks[0].text, "");
}

#[test]
fn an_embed_target_that_two_files_carry_is_not_remapped() {
    let (source_dir, copy_dir, destination_dir) = (Temp::new(), Temp::new(), Temp::new());
    let (host, target) = {
        let source = source_dir.open();
        let target = source.create_page(None, "Zulu city".into()).unwrap();
        let host = source.create_page(None, "Alpha trip".into()).unwrap();
        source
            .apply(host.clone(), embed_op("e", "", Some(target.as_str())))
            .unwrap();
        source.flush().unwrap();
        (host, target)
    };
    // A second file with the target's exported id makes that id ambiguous.
    let pages = source_dir.0.join("pages");
    fs::create_dir_all(&copy_dir.0).unwrap();
    let mut target_file = None;
    for e in fs::read_dir(&pages).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_owned();
        fs::copy(&p, copy_dir.0.join(&name)).unwrap();
        if p.to_string_lossy().contains(target.as_str()) {
            target_file = Some(p);
        }
    }
    let target_file = target_file.expect("target file");
    fs::copy(&target_file, copy_dir.0.join("Zulu copy.md")).unwrap();

    let destination = destination_dir.open();
    destination
        .import_markdown(copy_dir.0.to_string_lossy().into_owned(), None)
        .unwrap();
    destination.flush().unwrap();
    let host_new = destination.search("Alpha trip".into(), 20).unwrap()[0]
        .id
        .clone();
    let loaded = destination.load_page(host_new).unwrap();
    assert_ne!(host, loaded.id);
    assert_eq!(loaded.blocks[0].target.as_deref(), Some(target.as_str()));
}
