---
type: index
summary: "The Rust core (vault_core): the data model, state, search queries, the conflict format and folder mounts."
parent: "[[index]]"
status: current
---

# Core

These pages describe the Rust core that holds the vault: how pages, blocks and properties are modelled and stored, how search works, how conflicts are represented, and how PC folders are mounted. Plans, proposals and decisions are in the internal wiki (notes vault).

<!-- wiki:children -->
- [Data model](data-model.md): Everything is a page with blocks and properties; collections group pages; views are queries plus renderers; markdown is the file format.
- [Core (state)](core-state.md): The built Rust core (vault_core): Loro page documents, properties, collections and queries, trash, links, attachments, history, import, index, bench.
- [Search queries and live queries](search-queries.md): Search operators (date:, tag:, in:, property filters) parsed into one query spec, filtered search inside the 20 ms budget, and live queries pushed after index commits.
- [Conflict format v2](conflict-format.md): Approved Block, Subtree and Page conflicts, edit-beats-delete, synced resolutions and undo, journal repair log and vault-wide auto-resolve.
- [Folder mounts](mounts.md): Any PC folder can be mounted at a vault path; devices see one logical vault, with no symlinks.
- [Triage](triage.md): Merge proposals for duplicate pages, the merge and its undo, and the AI run that makes them.
<!-- /wiki:children -->
