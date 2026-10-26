//! Data types as Dart sees them. Plain structs and field-less enums only
//! (no enums with data, which would need `freezed` on the Dart side), so a
//! core enum with data becomes a struct with a kind and the fields of every
//! variant. Conversions live in `crate::convert`.
//!
//! - Mark offsets are UTF-16 code units (Dart's string unit); the core counts
//!   Unicode scalar values.
//! - Block kinds are the core's lowercase names (`"paragraph"`,
//!   `"heading1"`, ...), so the Dart block registry is keyed by string.
//! - A page's block tree comes back flattened (pre-order with depths).

use flutter_rust_bridge::frb;

/// Selects a type-data list item. Supply exactly one of `index` or `id`.
/// An id matches a string element or a map element's string `id` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDataListItem {
    pub index: Option<u32>,
    pub id: Option<String>,
}

// ------------------------------------------------------------------ pages

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BMarkKind {
    Bold,
    Italic,
    Strike,
    Code,
    Link,
    /// An inline mention of another page; `url` is the page id.
    PageLink,
    Underline,
    /// Text or background colour; `url` is the colour name (`red`,
    /// `red_background`).
    Color,
    /// A date mention; `url` is the ISO date.
    Date,
    /// A dynamic search chip; `url` is the query spec string.
    Query,
}

/// Offsets in UTF-16 code units. `url` is the link URL (`Link`), the page
/// id (`PageLink`), the colour (`Color`), the date (`Date`) or the query
/// spec string (`Query`), empty otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BMark {
    pub start: u32,
    pub end: u32,
    pub kind: BMarkKind,
    pub url: String,
}

/// One block of the flattened tree (pre-order); `depth` 0 is the top level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatBlock {
    pub id: String,
    /// The core's kind name: paragraph, heading1, heading2, heading3,
    /// bulleted, numbered, todo, quote, code, divider, linktopage, image,
    /// file, bookmark, toggle, toggleheading1..3, callout, columnlist,
    /// column, toc, database.
    pub kind: String,
    pub text: String,
    pub marks: Vec<BMark>,
    pub checked: bool,
    pub depth: u32,
    /// Page id, attachment id or URL for link, image, file and bookmark
    /// blocks; the icon of a callout.
    pub target: Option<String>,
    /// Block colour (`red`, `red_background`, ...), None for the default.
    pub color: Option<String>,
    /// Image width in percent of the text column, None for the full width.
    pub width: Option<u32>,
}

/// A page with its blocks and metadata.
#[derive(Debug, Clone)]
pub struct PageData {
    pub id: String,
    pub title: String,
    pub blocks: Vec<FlatBlock>,
    pub parent: Option<String>,
    pub icon: Option<String>,
    pub cover: Option<String>,
    /// The page's space colour, if one is assigned.
    pub color: Option<String>,
    /// A stable purpose such as `journal`, independent of the title.
    pub role: Option<String>,
    pub properties: Vec<Property>,
    /// `document`, `database`, or another type a mod defines (kept as is).
    pub page_type: String,
    pub favorite: bool,
    pub in_trash: bool,
    pub created_ms: i64,
    pub updated_ms: i64,
    /// Display options (they sync with the page).
    pub prefs: PagePrefsData,
    /// Time spent in the core's load plus flattening, in microseconds.
    pub load_us: u32,
}

/// A page's display options (the core's `PagePrefs`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PagePrefsData {
    pub small_text: bool,
    pub full_width: bool,
    pub locked: bool,
    /// Cover image position in per mille (0 top, 1000 bottom); None is the middle.
    pub cover_position: Option<u32>,
}

/// One row of a page list (the core's `PageSummary`).
#[derive(Debug, Clone)]
pub struct PageRow {
    pub id: String,
    pub title: String,
    pub parent: Option<String>,
    pub updated_ms: i64,
    pub created_ms: i64,
    /// Child pages not in the trash.
    pub child_count: u32,
    pub icon: Option<String>,
    /// `document`, `database`, or another type a mod defines (kept as is).
    pub page_type: String,
    pub favorite: bool,
    pub properties: Vec<Property>,
}

#[derive(Debug, Clone)]
#[frb]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub fuzzy: bool,
    pub corrected: Option<String>,
    /// The filter values this hit satisfied (`due: 2026-10-12`, `tag: work`),
    /// so filter-only results have something to show. Empty without filters.
    #[frb(default = "const []")]
    pub matched_filters: Vec<MatchedFilterValue>,
    /// `vault` for pages, or the external source that owns the row.
    pub source: String,
    /// The page type of a page, or the external row's type (for example `chat`).
    pub page_type: String,
    /// The external row's opaque target JSON; None for pages.
    pub target: Option<String>,
}

/// A row an external search source (for example agent chats) asks the vault
/// to hold and search. Its id is local to the source.
#[derive(Debug, Clone)]
#[frb]
pub struct ExternalSearchRow {
    pub id: String,
    pub title: String,
    /// Plain text that is searched.
    pub body: String,
    pub page_type: String,
    /// Opaque JSON the source uses to open the row; the vault does not read it.
    pub target: Option<String>,
    pub updated_ms: i64,
}

/// Search results and whether broad input was narrowed to page titles.
#[derive(Debug, Clone)]
pub struct SearchResults {
    pub hits: Vec<Hit>,
    pub narrowed: bool,
}

// ------------------------------------------------------------ search queries

/// One actual value that satisfied a filter operator on a hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedFilterValue {
    /// The operator's key, such as `due`, `tag` or the property's name.
    pub key: String,
    pub value: String,
}

/// A free-text term: a word or a `"phrase"`, with `exclude` for a leading `-`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchTerm {
    pub value: String,
    pub phrase: bool,
    pub exclude: bool,
}

/// A filter operator such as `tag:work`, `-in:"Projects"` or `due:<today`.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchOperator {
    pub key: String,
    /// Canonical, quote-aware comma alternatives; page references use ids.
    pub value: String,
    /// Written with a leading `-`.
    pub negated: bool,
    /// A property key rather than a built-in operator.
    pub property: bool,
    /// The first resolved page id, or the property key for one `has:<prop>`.
    pub resolved_id: Option<String>,
    /// The compiled filter condition, if the operator becomes one.
    pub filter: Option<Filter>,
    /// A date operator whose range is in the future (`+7d`).
    pub date_future: bool,
}

/// A parsed search: its free-text terms, operators, sort and warnings.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchSpec {
    pub terms: Vec<SearchTerm>,
    pub operators: Vec<SearchOperator>,
    /// The `sort:` key (`updated`, `created`, `title` or a property key).
    pub sort: Option<String>,
    /// Problems in the query. Ignored parts of the query are named here.
    pub warnings: Vec<String>,
}

impl SearchSpec {
    /// Canonical query text, preserving resolved page ids.
    pub fn to_query_string(&self) -> Result<String, String> {
        Ok(crate::convert::spec_to(self.clone())?.to_query_string())
    }
}

/// Which variant of the core's `TokenKind` a [`TokenKind`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchTokenKind {
    Text,
    Phrase,
    Exclude,
    Operator,
    Error,
}

/// What a query span is. Fields by kind: Text and Phrase: `value`; Exclude:
/// `value`, `phrase`; Operator: `key`, `value`, `resolved_id`; Error:
/// `message`. Unused fields are empty (`""`, `false`, `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenKind {
    pub kind: SearchTokenKind,
    pub value: String,
    pub phrase: bool,
    pub key: String,
    pub resolved_id: Option<String>,
    pub message: String,
}

/// One span of the query text. `start` and `end` are the byte offsets of
/// the span in the query string (UTF-8), so `end` is exclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub start: u32,
    pub end: u32,
    pub kind: TokenKind,
}

/// A query read into its spec and its spans (see `Vault::parse_search`).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedQuery {
    pub spec: SearchSpec,
    pub tokens: Vec<Token>,
}

/// Results of a parsed search. `total_bounded` saturates at 201 ("200+").
#[derive(Debug, Clone)]
pub struct SearchSpecResults {
    pub hits: Vec<Hit>,
    pub total_bounded: u32,
    pub narrowed: bool,
    pub warnings: Vec<String>,
}

/// A completion for the cursor. Replaces the byte range `[start, end)` of the
/// query with `value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchCompletion {
    pub value: String,
    /// The text the completion list shows for `value`.
    pub label: String,
    pub start: u32,
    pub end: u32,
    /// The resolved page id for a page-title completion.
    pub resolved_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Insert,
    Delete,
    Move,
    SetText,
    SetKind,
    SetChecked,
    SetTarget,
    SetColor,
    SetWidth,
}

/// One edit of a page's block tree. Which fields matter depends on `op`
/// (see vault_core::BlockOp). `parent`/`after` None mean top level / first
/// child. `kind` is a core kind name (Insert, SetKind).
#[derive(Debug, Clone)]
pub struct BOp {
    pub op: OpKind,
    pub id: String,
    pub parent: Option<String>,
    pub after: Option<String>,
    pub kind: String,
    pub text: String,
    pub marks: Vec<BMark>,
    pub checked: bool,
    pub target: Option<String>,
    pub color: Option<String>,
    pub width: Option<u32>,
}

// ------------------------------------------------------------- properties

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    Text,
    Number,
    Date,
    Checkbox,
    Select,
    MultiSelect,
    Status,
    Relation,
    Url,
    Email,
    Phone,
    CreatedTime,
    UpdatedTime,
    /// Attachments (`ids` of the value).
    Files,
    /// Computed from `formula` in the config; read only.
    Formula,
    /// Computed from `rollupRelation`, `rollupProperty`, `rollupFunction`
    /// in the config; read only.
    Rollup,
}

/// Which variant of the core's `PropValue` a [`PropValue`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Empty,
    Text,
    Number,
    Date,
    Checkbox,
    Select,
    MultiSelect,
    Status,
    Relation,
    Url,
    /// Attachment ids in `ids`.
    Files,
    /// A formula or rollup that failed: the message in `text`.
    Error,
}

/// A property value. Fields by kind: Text, Url, Select, Status: `text`
/// (select and status: the option id); Number: `number`; Date: `start_ms`,
/// `end_ms`, `has_time`; Checkbox: `checked`; MultiSelect, Relation: `ids`.
#[derive(Debug, Clone, PartialEq)]
pub struct PropValue {
    pub kind: ValueKind,
    pub text: String,
    pub number: f64,
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    pub has_time: bool,
    pub checked: bool,
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub key: String,
    pub value: PropValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectOption {
    /// Empty in `set_schema` = assign one.
    pub id: String,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropDef {
    /// Empty in `set_schema` = assign one.
    pub key: String,
    pub name: String,
    pub kind: PropKind,
    pub options: Vec<SelectOption>,
    /// Interface settings (JSON), stored and synced by the core.
    pub config: String,
}

// ------------------------------------------------------- views and queries

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    Table,
    List,
    Board,
    Calendar,
    Gallery,
    Timeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    And,
    Or,
    Cond,
}

/// A filter tree: `And`/`Or` use `children`, `Cond` uses `cond`.
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub kind: FilterKind,
    pub children: Vec<Filter>,
    pub cond: Option<Condition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    Is,
    IsNot,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    IsEmpty,
    IsNotEmpty,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
    Before,
    After,
    OnOrBefore,
    OnOrAfter,
    WithinRange,
    Checked,
    Unchecked,
}

/// `prop` is a property key, `$title`, `$created`, `$updated`, or `*`.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub prop: String,
    pub op: FilterOp,
    pub value: PropValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortSpec {
    pub prop: String,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewDef {
    /// Empty in `add_view` = assign one.
    pub id: String,
    pub name: String,
    pub kind: ViewKind,
    pub filter: Option<Filter>,
    pub sorts: Vec<SortSpec>,
    pub group_by: Option<String>,
    pub date_prop: Option<String>,
    pub visible_props: Vec<String>,
    pub card_cover: Option<String>,
    /// Interface settings (JSON), stored and synced by the core.
    pub config: String,
}

#[derive(Debug, Clone)]
pub struct QueryResult {
    pub rows: Vec<PageRow>,
    pub total: u32,
}

/// One board column.
#[derive(Debug, Clone)]
pub struct GroupRow {
    pub value: PropValue,
    pub label: String,
    pub color: Option<String>,
    pub count: u32,
}

#[derive(Debug, Clone)]
pub struct RangeEntry {
    pub page: PageRow,
    pub prop: String,
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    pub has_time: bool,
}

// --------------------------------------------------------------- agenda

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusGroup {
    Todo,
    InProgress,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgendaRole {
    Auto,
    Tasks,
    Events,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgendaSettings {
    pub role: AgendaRole,
    pub date_prop: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgendaKind {
    Task,
    Event,
    Mention,
}

/// Transient date-mark identity. Offsets count Unicode scalar values in the
/// core block, not Dart UTF-16 units; pass the reference back unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateMarkRef {
    pub start: u32,
    pub end: u32,
    pub iso: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgendaItem {
    pub page: String,
    pub block: Option<String>,
    /// Present for a dated to-do whose source mark still matches the item.
    pub date_mark: Option<DateMarkRef>,
    pub title: String,
    pub kind: AgendaKind,
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    pub has_time: bool,
    pub done: bool,
    pub status: Option<String>,
    pub status_group: Option<String>,
    pub status_color: Option<String>,
    pub calendar: Option<String>,
    pub assignee: Option<String>,
    pub collection: Option<String>,
    pub space: Option<String>,
    pub space_color: Option<String>,
    pub icon: Option<String>,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressKind {
    Checklist,
    Tasks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub page: String,
    pub title: String,
    pub icon: Option<String>,
    pub kind: ProgressKind,
    pub done: u32,
    pub total: u32,
    pub space: Option<String>,
    pub space_color: Option<String>,
    pub updated_ms: i64,
}

// ------------------------------------------------------------- calendars

/// A named calendar for task notes. Both switches are independent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarCategory {
    pub id: String,
    pub name: String,
    pub color: String,
    pub show_in_calendar: bool,
    pub show_in_today: bool,
}

/// Calendar visibility for one database; an absent entry means visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarDatabase {
    pub id: String,
    pub show_in_calendar: bool,
}

/// The calendar categories and per-database calendar visibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarSettings {
    pub calendars: Vec<CalendarCategory>,
    pub databases: Vec<CalendarDatabase>,
}

// -------------------------------------------------- mounts

/// A folder of this device shown as the children of page `page`.
#[derive(Debug, Clone)]
pub struct MountInfo {
    pub page: String,
    pub folder: String,
}

// -------------------------------------------------- trash, files, history

#[derive(Debug, Clone)]
pub struct TrashedPage {
    pub page: PageRow,
    pub trashed_ms: i64,
}

#[derive(Debug, Clone)]
pub struct AttachmentInfo {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: i64,
}

#[derive(Debug, Clone)]
pub struct VersionInfo {
    pub id: String,
    pub time_ms: i64,
    pub device: String,
    pub summary: String,
    pub changes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    PageChanged,
    PageCreated,
    PageDeleted,
    TreeChanged,
    IndexCommitted,
}

/// One change-feed entry. Index commits carry `generation` and `pages`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[frb]
pub struct Change {
    pub kind: ChangeKind,
    pub page: Option<String>,
    pub generation: Option<u64>,
    #[frb(default = "const []")]
    pub pages: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ChangeBatch {
    pub seq: u32,
    pub changes: Vec<Change>,
    /// The feed cannot say what changed: reload everything.
    pub reset: bool,
}

#[derive(Debug, Clone)]
pub struct ImportReport {
    pub pages: u32,
    pub attachments: u32,
    pub unresolved_links: u32,
    pub top_pages: Vec<String>,
}

// -------------------------------------------------------------- sync hooks

#[derive(Debug, Clone)]
pub struct PageVersion {
    pub page: String,
    pub version: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PageUpdate {
    pub page: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SyncImport {
    pub created: u32,
    pub changed: u32,
    pub conflicts: u32,
    pub skipped: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictScopeKind { Block, Subtree, Page }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictScope {
    pub kind: ConflictScopeKind,
    /// Block id or subtree root; None for page scope.
    pub block: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictStateKind { Present, Deleted, Trashed, Purged }

/// Complete scoped content, flattened in pre-order with depths, including
/// every child. Frontiers on the side remain the source of truth.
#[derive(Debug, Clone)]
pub struct ConflictSideState {
    pub kind: ConflictStateKind,
    pub content: Vec<FlatBlock>,
    pub trashed_ms: Option<i64>,
    pub tombstone_version: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictAnchor {
    pub parent: Option<String>,
    pub previous: Option<String>,
}

/// One complete side. `block` is a legacy display overlay for block scope;
/// subtree and page scopes use `state.content`.
#[derive(Debug, Clone)]
pub struct ConflictSide {
    pub device: String,
    pub local: bool,
    pub time_ms: i64,
    pub frontiers: Option<Vec<u8>>,
    pub state: ConflictSideState,
    pub anchor: Option<ConflictAnchor>,
    pub block: Option<FlatBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictStatus { Pending, Resolved }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionBy { User, Auto }

#[derive(Debug, Clone)]
pub struct ConflictBeforeState {
    pub kind: ConflictStateKind,
    pub trashed_ms: Option<i64>,
    pub tombstone_version: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct AppliedResolution {
    pub time_ms: i64,
    /// The applied choice; None for records created before choices were stored.
    pub resolution: Option<Resolution>,
    /// Undo time, retained in the log and history after the application is consumed.
    pub undone_at: Option<i64>,
    pub by: ResolutionBy,
    /// Present only for an automatic resolution.
    pub rule: Option<String>,
    pub before: Vec<u8>,
    /// None means the page was live before applying the resolution.
    pub before_state: Option<ConflictBeforeState>,
    pub fallback: bool,
    pub restored_ancestors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Conflict {
    pub id: String,
    pub format: u32,
    pub page: String,
    /// Legacy display key: block id or subtree root, empty for page scope.
    pub block: String,
    pub time_ms: i64,
    pub scope: ConflictScope,
    pub sides: Vec<ConflictSide>,
    pub status: ConflictStatus,
    pub applied: Vec<AppliedResolution>,
    pub undoable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionKind {
    /// Confirm the scope as it is on the page.
    Keep,
    /// Restore the complete scope as it was on side `side`.
    UseSide,
}

#[derive(Debug, Clone)]
pub struct Resolution {
    pub kind: ResolutionKind,
    pub side: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictDiffChange { Unchanged, Modified, Added, Removed }

/// Offsets count UTF-16 code units, matching Dart strings and BMark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictDiffSpan {
    pub start: u32,
    pub end: u32,
    pub change: ConflictDiffChange,
}

/// Flattened diff tree; block.depth retains the source hierarchy.
#[derive(Debug, Clone)]
pub struct ConflictDiffBlock {
    pub block: FlatBlock,
    pub change: ConflictDiffChange,
    pub spans: Vec<ConflictDiffSpan>,
}

#[derive(Debug, Clone)]
pub struct ConflictDiffSide {
    pub device: String,
    pub local: bool,
    pub state: ConflictSideState,
    pub blocks: Vec<ConflictDiffBlock>,
}

#[derive(Debug, Clone)]
pub struct ConflictDiff {
    pub scope: ConflictScope,
    pub sides: Vec<ConflictDiffSide>,
}

// --------------------------------------------------------------- vault_sync

/// Timings of the background sync loop (see vault_sync::SyncOptions).
#[derive(Debug, Clone)]
pub struct SyncOptions {
    pub debounce_ms: u32,
    pub max_delay_ms: u32,
    pub interval_ms: u32,
    pub long_poll_ms: u32,
    pub prefetch_attachments: bool,
}

/// What one sync did.
#[derive(Debug, Clone, Default)]
pub struct SyncReport {
    pub full: bool,
    pub rounds: u32,
    pub created: u32,
    pub changed: u32,
    pub conflicts: u32,
    pub pushed: u32,
    pub purged: u32,
    pub blobs_sent: u32,
    pub bytes_down: i64,
    pub bytes_up: i64,
}

/// The state of the background sync.
#[derive(Debug, Clone, Default)]
pub struct SyncStatus {
    /// The background loop is started.
    pub running: bool,
    /// A sync is in progress.
    pub syncing: bool,
    pub last_ok_ms: Option<i64>,
    pub last_error: Option<String>,
    pub last_report: Option<SyncReport>,
    pub syncs: i64,
}

/// A device paired with the embedded hub.
#[derive(Debug, Clone)]
pub struct PairedDevice {
    pub label: String,
    pub device: Option<String>,
    pub created_ms: i64,
}

/// A certificate the user trusted for one hub origin. Every hub client
/// (sync, the agent and host relay sockets) takes the device's list.
#[derive(Debug, Clone)]
pub struct TrustedHubCertificate {
    /// `scheme://host:port` it was trusted for.
    pub origin: String,
    /// SHA-256 of the hub's end-entity certificate (hex, any case, with or
    /// without `:`): accepted for an unknown issuer.
    pub sha256: String,
    /// DER of the certificate that issued it, when the hub sent one: a
    /// trust anchor for `origin` only, so a re-minted certificate from the
    /// same CA (a TLS-inspecting proxy) is accepted too.
    pub issuer_der: Option<Vec<u8>>,
}

impl TrustedHubCertificate {
    pub(crate) fn to_sync(list: Vec<TrustedHubCertificate>) -> Vec<vault_sync::tls::TrustedCertificate> {
        list.into_iter()
            .map(|t| vault_sync::tls::TrustedCertificate {
                origin: t.origin,
                sha256: t.sha256,
                issuer_der: t.issuer_der,
            })
            .collect()
    }
}

/// A certificate a hub presented, for showing it before trusting it.
#[derive(Debug, Clone)]
pub struct CertificateInfo {
    /// RFC 4514 style, for example `CN=hub.example,O=Example`. For a
    /// certificate that could not be parsed, `unparsable certificate: `
    /// and the reason, with only `sha256` set.
    pub subject: String,
    pub issuer: String,
    /// Validity start and end (ms since the epoch).
    pub not_before_ms: i64,
    pub not_after_ms: i64,
    /// SHA-256 of the certificate, uppercase hex pairs separated by `:`
    /// (the value to pin).
    pub sha256: String,
    /// Serial number, uppercase hex.
    pub serial: String,
    /// Subject and issuer are the same name.
    pub self_signed: bool,
}
