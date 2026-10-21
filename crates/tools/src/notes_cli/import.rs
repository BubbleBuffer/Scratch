//! Rerunnable import of Claude's project memory files through the memory tools.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use serde_yaml::Value as Yaml;

use super::{tool_text, Client};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectMap {
    prefixes: Vec<String>,
    #[serde(default)]
    overrides: BTreeMap<String, String>,
    fallback: String,
}

impl ProjectMap {
    fn project(&self, slug: &str) -> Result<String> {
        let project = if let Some(project) = self.overrides.get(slug) {
            project.as_str()
        } else if slug.contains("-scratch-") {
            &self.fallback
        } else {
            self.prefixes
                .iter()
                .filter(|prefix| slug.starts_with(prefix.as_str()))
                .max_by_key(|prefix| prefix.len())
                .map(|prefix| &slug[prefix.len()..])
                .unwrap_or(&self.fallback)
        };
        ensure_segment(project).context("mapped project is not a valid memory path segment")?;
        Ok(project.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Memory {
    name: String,
    description: String,
    kind: String,
    source: Option<String>,
    body: String,
}

#[derive(Debug)]
struct Candidate {
    file: PathBuf,
    modified: SystemTime,
    source_text: String,
    memory: Memory,
}

struct Selected {
    key: String,
    memory: Memory,
    conflicts: Vec<(PathBuf, bool)>,
}

struct Existing {
    path: String,
    memory: Memory,
}

#[derive(Default)]
struct Counts {
    new: usize,
    updated: usize,
    unchanged: usize,
    conflicts: usize,
}

fn ensure_segment(segment: &str) -> Result<()> {
    let bytes = segment.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 80
        || !bytes[0].is_ascii_alphanumeric()
        || !bytes[1..]
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    {
        bail!("expected 1 to 80 letters, digits, '.', '_' or '-', starting with a letter or digit");
    }
    Ok(())
}

fn utf8_name(path: &Path) -> Result<&str> {
    path.file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("non-UTF-8 source filename: {}", path.display()))
}

fn yaml_key<'a>(value: &'a Yaml, key: &str) -> Option<&'a Yaml> {
    value.as_mapping()?.get(Yaml::String(key.to_string()))
}

fn required_text(value: Option<&Yaml>, key: &str) -> Result<String> {
    match value.and_then(Yaml::as_str) {
        Some(text) if !text.trim().is_empty() => Ok(text.to_string()),
        _ => {
            bail!("missing or malformed required frontmatter field {key} (expected nonempty text)")
        }
    }
}

/// Keep extra metadata as YAML in the body, with its original structure.
fn body_with_metadata(body: &str, frontmatter: &Yaml) -> Result<String> {
    let mut extras = frontmatter.clone();
    let root = extras.as_mapping_mut().expect("frontmatter is a mapping");
    for key in ["name", "description", "type", "source"] {
        root.remove(Yaml::String(key.to_string()));
    }
    let metadata_key = Yaml::String("metadata".to_string());
    if let Some(metadata) = root.get_mut(&metadata_key).and_then(Yaml::as_mapping_mut) {
        let removed_type = metadata.remove(Yaml::String("type".to_string())).is_some();
        if removed_type && metadata.is_empty() {
            root.remove(&metadata_key);
        }
    }
    if root.is_empty() {
        return Ok(body.to_string());
    }
    let mut yaml = serde_yaml::to_string(&extras)
        .map_err(|_| anyhow!("cannot serialize extra frontmatter metadata"))?;
    // The vault parser accepts either fence marker, but not a longer run as
    // an escape. Select a marker absent from metadata's potential closing lines.
    let backticks = yaml
        .lines()
        .any(|line| line.trim_start().starts_with("```"));
    let tildes = yaml
        .lines()
        .any(|line| line.trim_start().starts_with("~~~"));
    let fence = match (backticks, tildes) {
        (false, _) => "```",
        (true, false) => "~~~~",
        (true, true) => {
            // JSON is valid YAML and escapes newlines inside strings. Use it
            // only if its parsed value is exactly the same, preserving tags
            // and non-string keys rather than silently converting them.
            let escaped = serde_json::to_string_pretty(&extras)
                .map_err(|_| anyhow!("cannot safely fence extra frontmatter metadata"))?;
            let roundtrip: Yaml = serde_yaml::from_str(&escaped)
                .map_err(|_| anyhow!("cannot safely fence extra frontmatter metadata"))?;
            if roundtrip != extras {
                bail!("cannot safely fence extra frontmatter metadata without changing its YAML value");
            }
            yaml = format!("{escaped}\n");
            "```"
        }
    };
    let separator = if body.is_empty() || body.ends_with("\n\n") {
        ""
    } else if body.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    Ok(format!(
        "{body}{separator}## Imported metadata\n\n{fence}yaml\n{}{fence}\n",
        yaml
    ))
}

fn parse_frontmatter(header: &str) -> Result<Yaml> {
    match serde_yaml::from_str(header) {
        Ok(yaml) => Ok(yaml),
        Err(original_error) => {
            // Claude sometimes writes a raw single-line description such as
            // `description: Use disk C:`. YAML treats that final colon (or
            // `: ` inside the text) as mapping syntax. Repair only the two
            // known top-level text fields, after strict parsing has failed.
            let mut repaired = String::new();
            let mut changed = false;
            for line in header.lines() {
                let scalar = ["name:", "description:"].iter().find_map(|prefix| {
                    line.strip_prefix(*prefix).and_then(|value| {
                        if !value.is_empty() && !value.starts_with(char::is_whitespace) {
                            return None;
                        }
                        let value = value.trim();
                        let first = value.chars().next()?;
                        if "'\"|>[]{}!&*#?-,:%@`".contains(first)
                            || !(value.contains(": ") || value.ends_with(':'))
                        {
                            return None;
                        }
                        Some((*prefix, value))
                    })
                });
                if let Some((prefix, value)) = scalar {
                    repaired.push_str(prefix);
                    repaired.push(' ');
                    repaired.push_str(
                        &serde_json::to_string(value).expect("text scalar is serializable"),
                    );
                    changed = true;
                } else {
                    repaired.push_str(line);
                }
                repaired.push('\n');
            }
            if changed {
                if let Ok(yaml) = serde_yaml::from_str(&repaired) {
                    return Ok(yaml);
                }
            }
            // serde_yaml's Display can include the offending private line.
            match original_error.location() {
                Some(location) => bail!(
                    "malformed YAML frontmatter at line {}, column {}",
                    location.line() + 1,
                    location.column()
                ),
                None => bail!("malformed YAML frontmatter"),
            }
        }
    }
}

fn parse_source(text: &str) -> Result<Memory> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let normalized = text.replace("\r\n", "\n");
    let mut lines = normalized.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        bail!("missing YAML frontmatter opening delimiter");
    }
    let start = normalized.find('\n').unwrap_or(normalized.len()) + 1;
    let mut end = start;
    let mut frontmatter_end = None;
    for line in lines {
        if line.trim_end() == "---" {
            frontmatter_end = Some((end, end + line.len()));
            break;
        }
        end += line.len();
    }
    let (end, body_start) =
        frontmatter_end.ok_or_else(|| anyhow!("missing YAML frontmatter closing delimiter"))?;
    let yaml = parse_frontmatter(&normalized[start..end])?;
    if !yaml.is_mapping() {
        bail!("frontmatter must be a mapping");
    }
    let name = required_text(yaml_key(&yaml, "name"), "name")?;
    let description = required_text(yaml_key(&yaml, "description"), "description")?;
    let kind = required_text(
        yaml_key(&yaml, "metadata")
            .and_then(|metadata| yaml_key(metadata, "type"))
            .or_else(|| yaml_key(&yaml, "type")),
        "metadata.type (or type)",
    )?;
    if !["user", "feedback", "project", "reference"].contains(&kind.as_str()) {
        bail!(
            "malformed required frontmatter type (expected user, feedback, project or reference)"
        );
    }
    Ok(Memory {
        name,
        description,
        kind,
        source: None,
        body: body_with_metadata(&normalized[body_start..], &yaml)?,
    })
}

fn sorted_entries(directory: &Path) -> Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(directory)
        .with_context(|| format!("cannot read source directory {}", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn sources(projects_dir: &Path, map: &ProjectMap) -> Result<Vec<Selected>> {
    let mut groups: BTreeMap<String, Vec<Candidate>> = BTreeMap::new();
    for directory in sorted_entries(projects_dir)? {
        if !directory.file_type()?.is_dir() {
            continue;
        }
        let project_path = directory.path();
        let slug = utf8_name(&project_path)?;
        let memory_dir = project_path.join("memory");
        if !memory_dir.exists() {
            continue;
        }
        for file in sorted_entries(&memory_dir)? {
            if !file.file_type()?.is_file() {
                continue;
            }
            let path = file.path();
            if path.extension().and_then(|x| x.to_str()) != Some("md")
                || utf8_name(&path)? == "MEMORY.md"
            {
                continue;
            }
            let project = map
                .project(slug)
                .with_context(|| format!("invalid project mapping for {}", path.display()))?;
            let stem = path
                .file_stem()
                .and_then(|x| x.to_str())
                .ok_or_else(|| anyhow!("invalid source filename {}", path.display()))?;
            ensure_segment(stem)
                .with_context(|| format!("invalid memory filename {}", path.display()))?;
            let key = format!("{project}/{stem}");
            let text = fs::read_to_string(&path)
                .with_context(|| format!("cannot read source {}", path.display()))?;
            let memory = parse_source(&text)
                .map_err(|error| anyhow!("cannot import {}: {error}", path.display()))?;
            let modified = file
                .metadata()?
                .modified()
                .with_context(|| format!("cannot read modification time for {}", path.display()))?;
            groups.entry(key).or_default().push(Candidate {
                file: path,
                modified,
                source_text: text,
                memory,
            });
        }
    }
    let mut selected = Vec::new();
    for (key, mut candidates) in groups {
        // Newest mtime wins; a tie chooses the lexicographically first path.
        candidates.sort_by(|a, b| {
            b.modified
                .cmp(&a.modified)
                .then_with(|| a.file.cmp(&b.file))
        });
        let conflict = candidates
            .iter()
            .skip(1)
            .any(|candidate| candidate.source_text != candidates[0].source_text);
        let conflicts = if conflict {
            candidates
                .iter()
                .enumerate()
                .map(|(i, candidate)| (candidate.file.clone(), i == 0))
                .collect()
        } else {
            Vec::new()
        };
        let candidate = candidates.remove(0);
        selected.push(Selected {
            key,
            memory: candidate.memory,
            conflicts,
        });
    }
    Ok(selected)
}

fn call(client: &Client, name: &str, args: Value) -> Result<String> {
    tool_text(&client.call(name, &args)?)
}

fn listed_paths(text: &str, empty: impl Fn(&str) -> bool) -> Result<Vec<String>> {
    if empty(text) {
        return Ok(Vec::new());
    }
    text.lines()
        .map(|line| {
            let path = line
                .strip_prefix("- ")
                .and_then(|line| line.split_once(" — ").map(|(path, _)| path))
                .ok_or_else(|| {
                    anyhow!("unexpected memory_list output; refusing an incomplete vault scan")
                })?;
            if path
                .split('/')
                .any(|segment| ensure_segment(segment).is_err())
            {
                bail!("unexpected path in memory_list output");
            }
            Ok(path.to_string())
        })
        .collect()
}

fn parse_existing(path: &str, text: &str) -> Result<Memory> {
    let text = text.replace("\r\n", "\n");
    let (header, body) = text
        .split_once("\n\n---\n")
        .ok_or_else(|| anyhow!("unexpected memory_read output for {path}"))?;
    let mut lines: Vec<&str> = header.lines().collect();
    if lines.first().copied() != Some(format!("# {path}").as_str()) {
        bail!("unexpected memory_read header for {path}");
    }
    // The final modified line is generated by the tool; source metadata now
    // lives in the body and is compared as part of its Markdown.
    if !lines
        .last()
        .is_some_and(|line| line.starts_with("- modified: "))
    {
        bail!("missing generated modified line in memory_read for {path}");
    }
    lines.pop();
    let mut fields = BTreeMap::<String, String>::new();
    let mut previous: Option<String> = None;
    for line in lines.into_iter().skip(1) {
        if let Some((key, value)) = line
            .strip_prefix("- ")
            .and_then(|line| line.split_once(": "))
        {
            let key = key.to_ascii_lowercase();
            if fields.insert(key.clone(), value.to_string()).is_some() {
                bail!("ambiguous repeated field in memory_read for {path}");
            }
            previous = Some(key);
        } else if let Some(key) = &previous {
            let value = fields.get_mut(key).expect("previous field exists");
            value.push('\n');
            value.push_str(line);
        } else {
            bail!("unexpected property output in memory_read for {path}");
        }
    }
    let name = fields
        .remove("name")
        .ok_or_else(|| anyhow!("missing name in memory_read for {path}"))?;
    let description = fields
        .remove("description")
        .ok_or_else(|| anyhow!("missing description in memory_read for {path}"))?;
    let kind = fields
        .remove("type")
        .ok_or_else(|| anyhow!("missing type in memory_read for {path}"))?;
    if !["user", "feedback", "project", "reference"].contains(&kind.as_str()) {
        bail!("memory {path} has an unreadable type; a moved memory may have properties that do not match its destination schema; repair the row/schema mismatch before importing to avoid duplicates");
    }
    Ok(Memory {
        name,
        description,
        kind,
        source: fields.remove("source"),
        body: if body == "(empty)" {
            String::new()
        } else {
            body.to_string()
        },
    })
}

fn scan(client: &Client) -> Result<(BTreeMap<String, Existing>, BTreeSet<String>)> {
    let projects = listed_paths(&call(client, "memory_list", json!({}))?, |text| {
        text == "No memory projects yet."
    })?;
    let mut by_source = BTreeMap::new();
    let mut occupied = BTreeSet::new();
    for project in projects {
        let paths = listed_paths(
            &call(client, "memory_list", json!({"path": project}))?,
            |text| text == format!("No memories in {project:?}."),
        )?;
        for path in paths {
            if !occupied.insert(path.clone()) {
                bail!("duplicate memory path in vault listing: {path}");
            }
            let memory =
                parse_existing(&path, &call(client, "memory_read", json!({"path": path}))?)?;
            if let Some(source) = memory.source.as_ref().filter(|value| !value.is_empty()) {
                if let Some(previous) = by_source.get(source) {
                    let previous: &Existing = previous;
                    bail!("ambiguous import source key on memories {} and {path}; no writes performed", previous.path);
                }
                by_source.insert(source.clone(), Existing { path, memory });
            }
        }
    }
    Ok((by_source, occupied))
}

fn same_memory(wanted: &Memory, current: &Memory, source: &str) -> bool {
    wanted.name == current.name
        && wanted.description == current.description
        && wanted.kind == current.kind
        && current.source.as_deref() == Some(source)
        && vault_core::normalize_markdown(&wanted.body)
            == vault_core::normalize_markdown(&current.body)
}

/// Import all project memory files. With no client, only an offline dry run is
/// allowed and its counts assume the vault is empty.
pub fn run(
    client: Option<&Client>,
    projects_dir: &Path,
    map_path: &Path,
    dry_run: bool,
) -> Result<String> {
    if client.is_none() && !dry_run {
        bail!("a connected vault is required unless --dry-run is used");
    }
    let config = fs::read_to_string(map_path)
        .with_context(|| format!("cannot read project map {}", map_path.display()))?;
    let map: ProjectMap = serde_json::from_str(&config)
        .with_context(|| format!("invalid project map {}", map_path.display()))?;
    ensure_segment(&map.fallback).context("invalid fallback project in project map")?;
    for project in map.overrides.values() {
        ensure_segment(project).context("invalid override project in project map")?;
    }
    let selected = sources(projects_dir, &map)?;
    let (existing, occupied) = match client {
        Some(client) => scan(client)?,
        None => (BTreeMap::new(), BTreeSet::new()),
    };
    // Resolve every source before the first write. Never replace another
    // imported memory, or an untagged memory, just because its path collides.
    for item in &selected {
        if !existing.contains_key(&item.key) && occupied.contains(&item.key) {
            bail!("destination {} is occupied by a memory with a different or missing source key; no writes performed", item.key);
        }
    }
    let mut counts: BTreeMap<String, Counts> = BTreeMap::new();
    let mut conflicts = Vec::new();
    for item in selected {
        let project = item
            .key
            .split('/')
            .next()
            .expect("source key contains project");
        let count = counts.entry(project.to_string()).or_default();
        if !item.conflicts.is_empty() {
            count.conflicts += 1;
            for (file, chosen) in &item.conflicts {
                conflicts.push(format!(
                    "Conflict {}: {} {}",
                    item.key,
                    if *chosen { "selected" } else { "ignored" },
                    file.display()
                ));
            }
        }
        let old = existing.get(&item.key);
        if old.is_some_and(|old| same_memory(&item.memory, &old.memory, &item.key)) {
            count.unchanged += 1;
            continue;
        }
        if old.is_some() {
            count.updated += 1;
        } else {
            count.new += 1;
        }
        if dry_run {
            continue;
        }
        let client = client.expect("non-dry run requires client");
        let path = old.map(|old| old.path.as_str()).unwrap_or(&item.key);
        if old.is_none() {
            call(client, "memory_write", json!({"path": path, "name": item.memory.name, "description": item.memory.description, "type": item.memory.kind, "body": item.memory.body}))
                .with_context(|| format!("could not create imported memory {path}"))?;
            call(
                client,
                "memory_update",
                json!({"path": path, "fields": {"source": item.key}}),
            )
            .with_context(|| format!("created {path}, but could not set its import identity"))?;
        } else {
            call(client, "memory_update", json!({"path": path, "name": item.memory.name, "description": item.memory.description, "type": item.memory.kind, "body": item.memory.body, "fields": {"source": item.key}}))
                .with_context(|| format!("could not update imported memory {path}"))?;
        }
    }
    let mut report = Vec::new();
    if client.is_none() {
        report.push("Offline dry run: assuming an empty vault.".to_string());
    } else if dry_run {
        report.push("Dry run: no writes performed.".to_string());
    }
    for (project, count) in counts {
        report.push(format!(
            "{project}: new={} updated={} unchanged={} conflicts={}",
            count.new, count.updated, count.unchanged, count.conflicts
        ));
    }
    if report.is_empty() || report.last().is_some_and(|line| !line.contains("new=")) {
        report.push("No source memories found.".to_string());
    }
    if !conflicts.is_empty() {
        report.push("Conflicts use newest modification time; ties select the lexicographically first source path.".to_string());
        report.extend(conflicts);
    }
    Ok(report.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> ProjectMap {
        serde_json::from_str(r#"{"prefixes":["C--Users-Admin-","C--Users-Admin-Documents-Projects-"],"overrides":{"x-scratch-y":"kept"},"fallback":"unsorted"}"#).unwrap()
    }

    #[test]
    fn mapping_uses_longest_prefix_and_override_before_scratch() {
        assert_eq!(
            map()
                .project("C--Users-Admin-Documents-Projects-notes-app")
                .unwrap(),
            "notes-app"
        );
        assert_eq!(map().project("x-scratch-y").unwrap(), "kept");
        assert_eq!(
            map().project("C--Users-Admin-a-scratch-b").unwrap(),
            "unsorted"
        );
        assert_eq!(map().project("unknown").unwrap(), "unsorted");
    }

    fn imported_metadata(body: &str) -> Yaml {
        let (_, block) = body.rsplit_once("## Imported metadata\n\n").unwrap();
        let mut lines = block.lines();
        let fence = lines.next().unwrap().trim_end_matches("yaml");
        let yaml = lines
            .take_while(|line| *line != fence)
            .collect::<Vec<_>>()
            .join("\n");
        serde_yaml::from_str(&yaml).unwrap()
    }

    #[test]
    fn nested_metadata_keeps_structure_and_does_not_create_fields() {
        let memory = parse_source("---\nname: sample\ndescription: 'a: description'\nmetadata:\n  type: feedback\n  modified: 2026-10-14\n  inner:\n    node_type: thing\noriginSessionId: abc\n---\n\n# Body\n").unwrap();
        assert_eq!(memory.kind, "feedback");
        assert_eq!(memory.description, "a: description");
        assert!(memory
            .body
            .starts_with("\n# Body\n\n## Imported metadata\n\n```yaml\n"));
        let extras = imported_metadata(&memory.body);
        assert_eq!(extras, serde_yaml::from_str::<Yaml>("metadata:\n  modified: 2026-10-14\n  inner:\n    node_type: thing\noriginSessionId: abc\n").unwrap());
        assert!(memory.source.is_none());
    }

    #[test]
    fn metadata_without_extras_adds_no_block_and_unknown_keys_keep_their_names() {
        let minimal = parse_source("---\nname: a\ndescription: b\nmetadata:\n  type: reference\nsource: reserved\n---\nbody\n").unwrap();
        assert_eq!(minimal.body, "body\n");
        let extra = parse_source("---\nname: a\ndescription: b\ntype: reference\nsource: reserved\nimport_source_hash: original\nmetadata:\n  import_content_hash: retained\n---\nbody").unwrap();
        assert_eq!(
            imported_metadata(&extra.body),
            serde_yaml::from_str::<Yaml>(
                "import_source_hash: original\nmetadata:\n  import_content_hash: retained\n"
            )
            .unwrap()
        );
        let repeated_leaf = parse_source("---\nname: a\ndescription: b\ntype: project\na:\n  modified: first\nb:\n  modified: second\n---\nbody").unwrap();
        assert_eq!(
            imported_metadata(&repeated_leaf.body),
            serde_yaml::from_str::<Yaml>("a:\n  modified: first\nb:\n  modified: second\n")
                .unwrap()
        );
    }

    #[test]
    fn metadata_errors_do_not_echo_source_values() {
        let error =
            parse_source("---\nname: x\ndescription: private\ntype: SECRET_TYPE\n---\nSECRET_BODY")
                .unwrap_err()
                .to_string();
        assert!(!error.contains("SECRET"));
        let error = parse_source("---\nname: [SECRET_VALUE\n---\nSECRET_BODY")
            .unwrap_err()
            .to_string();
        assert!(!error.contains("SECRET"));
    }

    #[test]
    fn metadata_code_fences_and_nested_sequences_preserve_their_values() {
        let memory = parse_source("---\nname: a\ndescription: b\ntype: project\nmetadata:\n  snippets:\n    - |\n      ```\n      sample\n      ~~~\n    - nested: [one, two]\n---\nbody").unwrap();
        let expected: Yaml = serde_yaml::from_str("metadata:\n  snippets:\n    - |\n      ```\n      sample\n      ~~~\n    - nested: [one, two]\n").unwrap();
        assert_eq!(imported_metadata(&memory.body), expected);
    }

    #[test]
    fn claude_plain_scalar_colons_are_repaired_after_strict_yaml_failure() {
        let text = "---\nname: Synthetic: title\ndescription: Keep this synthetic path on disk C:\nmetadata:\n  type: feedback\n---\n\nUntouched body: [text]\n";
        let memory = parse_source(text).unwrap();
        assert_eq!(memory.name, "Synthetic: title");
        assert_eq!(memory.description, "Keep this synthetic path on disk C:");
        assert_eq!(memory.body, "\nUntouched body: [text]\n");
        let embedded = parse_source("---\nname: example\ndescription: Synthetic explanation: keep it literal\ntype: project\n---\nbody").unwrap();
        assert_eq!(
            embedded.description,
            "Synthetic explanation: keep it literal"
        );
    }

    #[test]
    fn scalar_compatibility_preserves_yaml_styles_and_rejects_broken_structure() {
        let quoted = parse_source(
            "---\nname: example\ndescription: 'Synthetic: quoted'\ntype: reference\n---\nbody",
        )
        .unwrap();
        assert_eq!(quoted.description, "Synthetic: quoted");
        let block = parse_source("---\nname: example\ndescription: |\n  Synthetic: block\n  Disk C:\ntype: reference\n---\nbody").unwrap();
        assert_eq!(block.description, "Synthetic: block\nDisk C:\n");
        for header in [
            "name: [PRIVATE_BROKEN\ndescription: Synthetic: raw\ntype: project\n",
            "name: example\ndescription: [PRIVATE_BROKEN: value\ntype: project\n",
            "name: example\ndescription: Synthetic: raw\nmetadata: {type: project\n",
        ] {
            let error = parse_source(&format!("---\n{header}---\nPRIVATE_BODY"))
                .unwrap_err()
                .to_string();
            assert!(error.contains("malformed YAML frontmatter"));
            assert!(!error.contains("PRIVATE"));
        }
    }

    #[test]
    fn existing_source_and_metadata_body_are_read_without_extra_schema_fields() {
        let memory = parse_existing("p/m", "# p/m\n- name: m\n- description: d\n- type: project\n- source: p/m\n- modified: 2026-10-14T02:00:00Z\n\n---\nbody\n\n## Imported metadata\n\n```\nmetadata:\n  modified: original\n```").unwrap();
        assert_eq!(memory.source.as_deref(), Some("p/m"));
        assert_eq!(
            imported_metadata(&memory.body)["metadata"]["modified"].as_str(),
            Some("original")
        );
    }

    #[test]
    fn unreadable_existing_type_reports_possible_move_without_private_values() {
        for kind in ["", "PRIVATE_INVALID_TYPE"] {
            let text = format!("# destination/moved\n- name: \n- description: \n- type: {kind}\n- modified: 2026-10-14T02:00:00Z\n\n---\nPRIVATE_BODY");
            let error = parse_existing("destination/moved", &text)
                .unwrap_err()
                .to_string();
            assert!(error.contains("destination/moved"));
            assert!(error.contains("destination schema"));
            assert!(!error.contains("PRIVATE"));
        }
    }

    #[test]
    fn markdown_comparison_includes_extra_metadata_and_detects_vault_edits() {
        let wanted = parse_source("---\nname: a\ndescription: b\ntype: project\nmetadata:\n  modified: original\n---\n_italic_\n").unwrap();
        let mut rendered = wanted.clone();
        rendered.body = vault_core::normalize_markdown(&wanted.body);
        rendered.source = Some("p/a".to_string());
        assert!(same_memory(&wanted, &rendered, "p/a"));
        let mut source_edit = wanted.clone();
        source_edit.body.push_str("changed");
        assert!(!same_memory(&source_edit, &rendered, "p/a"));
        let mut vault_edit = rendered.clone();
        vault_edit.body.push_str("\n\nchanged");
        assert!(!same_memory(&wanted, &vault_edit, "p/a"));
        vault_edit = rendered.clone();
        vault_edit.body = vault_edit
            .body
            .replace("modified: original", "modified: changed");
        assert!(!same_memory(&wanted, &vault_edit, "p/a"));
        rendered.source = Some("p/other".to_string());
        assert!(!same_memory(&wanted, &rendered, "p/a"));
    }
}
