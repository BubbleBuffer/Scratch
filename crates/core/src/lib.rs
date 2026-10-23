//! `vault_core`: vault storage, one Loro CRDT document per page, markdown
//! files written from it, and a SQLite index with full-text search.
//!
//! The public surface is [`Vault`], the plain data types in [`types`], the
//! single error type [`VaultError`] and [`seed::generate`]. Every public
//! function takes owned arguments and is synchronous; run it off the UI
//! thread. Design notes: `wiki/architecture/core-state.md`.

mod agenda;
mod agent_defaults;
mod attach;
mod blockdiff;
mod calendars;
mod collections;
mod computed;
mod conflict_archive;
mod conflict_detect;
mod conflict_diff;
mod conflict_policy;
mod conflict_resolution;
mod conflict_restore;
mod conflict_settings;
mod day_repair;
mod doc;
mod error;
mod feed;
mod feedback;
mod formula;
mod fts_rank;
mod fuzzy;
mod history;
mod ids;
mod import;
mod index;
mod live_query;
mod markdown;
mod mdio;
mod mdparse;
mod media;
mod mounts;
mod nodes;
mod order;
mod query;
mod search_api;
mod search_exec;
mod search_items;
pub mod search_ops;
mod search_path;
pub mod search_query;
pub mod seed;
#[cfg(feature = "storage-metrics")]
pub mod storage_metrics;
mod sync;
mod templates;
mod tree;
mod type_data;
pub mod types;
mod vault;

pub use agent_defaults::{
    AgentDefaults, AGENT_RUNNER_HUB, AGENT_RUNNER_THIS_DEVICE, AGENT_TOOL_SETS,
};
pub use conflict_diff::{
    ConflictDiff, ConflictDiffBlock, ConflictDiffChange, ConflictDiffSide, ConflictDiffSpan,
};
pub use conflict_settings::{CONFLICT_LOG_RETENTION_DAYS, CONFLICT_LOG_RETENTION_MS};
pub use error::{Result, VaultError};
pub use ids::{is_page_id, new_id, node_record_id, schedule_occurrence_id};
pub use live_query::LiveQuerySubscription;
/// Roles of the hidden system pages (`role:` in their markdown).
pub use markdown::ROLES as PAGE_ROLES;
pub use mdio::normalize_markdown;
pub use search_items::{ItemHit, ItemResults};
pub use search_query::{
    ParsedQuery, SearchExtras, SearchOperator, SearchSpec, SearchTerm, Token, TokenKind,
};
pub use sync::{sha256_hex, version_includes, PurgeRecord, RestoreRecord};
pub use type_data::ListItem;
pub use types::*;
pub use vault::Vault;
