//! Plain data types of the public API. All of them are owned, serde-friendly
//! and free of lifetimes and generics, so flutter_rust_bridge and Tauri
//! commands can carry them as they are.

use serde::{Deserialize, Serialize};

/// Opaque page id (a 26-character ULID-style string), stable across renames.
pub type PageId = String;
/// Opaque block id, chosen by the caller when inserting (any non-empty string).
pub type BlockId = String;
/// An attachment: the lowercase hex SHA-256 of its bytes.
pub type AttachmentId = String;

/// The page type of an ordinary page, and the default when none is stored.
pub const PAGE_TYPE_DOCUMENT: &str = "document";
/// The page type of a database: a page whose children are rows of its schema.
pub const PAGE_TYPE_DATABASE: &str = "database";

/// Serde default for `page_type`: a page without a stored type is a document.
pub(crate) fn default_page_type() -> String {
    PAGE_TYPE_DOCUMENT.to_string()
}

/// One row of the page list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageSummary {
    pub id: PageId,
    pub title: String,
    pub parent: Option<PageId>,
    pub updated_ms: i64,
    /// Number of child pages (not counting pages in the trash).
    pub child_count: u32,
    #[serde(default)]
    pub created_ms: i64,
    /// An emoji (or any short string), if set.
    #[serde(default)]
    pub icon: Option<String>,
    /// The page type: `document`, `database`, or another string a mod
    /// defines, kept as it is. A database has a schema and its children are
    /// rows.
    #[serde(default = "default_page_type")]
    pub page_type: String,
    #[serde(default)]
    pub favorite: bool,
    /// Stored properties, sorted by key.
    #[serde(default)]
    pub properties: Vec<Property>,
}

impl PageSummary {
    /// Whether the page is a database.
    pub fn is_database(&self) -> bool {
        self.page_type == PAGE_TYPE_DATABASE
    }
}

impl Page {
    /// Whether the page is a database.
    pub fn is_database(&self) -> bool {
        self.page_type == PAGE_TYPE_DATABASE
    }
}

/// A page with its full block tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    pub id: PageId,
    pub title: String,
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub parent: Option<PageId>,
    #[serde(default)]
    pub icon: Option<String>,
    /// The cover image, an attachment.
    #[serde(default)]
    pub cover: Option<AttachmentId>,
    /// Stored properties, sorted by key.
    #[serde(default)]
    pub properties: Vec<Property>,
    /// The page type (see [`PageSummary::page_type`]).
    #[serde(default = "default_page_type")]
    pub page_type: String,
    #[serde(default)]
    pub favorite: bool,
    /// The page or one of its ancestors is in the trash.
    #[serde(default)]
    pub in_trash: bool,
    #[serde(default)]
    pub created_ms: i64,
    #[serde(default)]
    pub updated_ms: i64,
    /// Display options (small text, full width, lock, cover position).
    #[serde(default)]
    pub prefs: PagePrefs,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatusGroup {
    Todo,
    InProgress,
    Complete,
}

/// Configured status groups take precedence over option order.
pub fn status_group(def: &PropDef, option_id: &str) -> StatusGroup {
    let config: serde_json::Value = serde_json::from_str(&def.config).unwrap_or_default();
    match config
        .get("groups")
        .and_then(|g| g.get(option_id))
        .and_then(|g| g.as_str())
    {
        Some("todo") => return StatusGroup::Todo,
        Some("inProgress") => return StatusGroup::InProgress,
        Some("complete") => return StatusGroup::Complete,
        _ => {}
    }
    if def
        .options
        .iter()
        .position(|o| o.id == option_id)
        .is_none_or(|i| i == 0)
    {
        StatusGroup::Todo
    } else if def.options.last().is_some_and(|o| o.id == option_id) {
        StatusGroup::Complete
    } else {
        StatusGroup::InProgress
    }
}

pub fn first_option_in(def: &PropDef, group: StatusGroup) -> Option<&SelectOption> {
    def.options
        .iter()
        .find(|o| status_group(def, &o.id) == group)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgendaRole {
    #[default]
    Auto,
    Tasks,
    Events,
    Off,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgendaSettings {
    pub role: AgendaRole,
    pub date_prop: Option<String>,
}

/// A named calendar for task notes. Both switches are independent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarCategory {
    pub id: String,
    pub name: String,
    pub color: String,
    pub show_in_calendar: bool,
    pub show_in_today: bool,
}

/// Calendar visibility for one database; an absent entry means visible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarDatabase {
    pub id: PageId,
    pub show_in_calendar: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarSettings {
    #[serde(default)]
    pub calendars: Vec<CalendarCategory>,
    #[serde(default)]
    pub databases: Vec<CalendarDatabase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgendaKind {
    Task,
    Event,
    Mention,
}

/// A transient reference to the inline date that produced a calendar task.
/// Offsets count Unicode scalar values; validate against the current document
/// before using this reference. It is never stored in the agenda index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateMarkRef {
    pub start: u32,
    pub end: u32,
    pub iso: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgendaItem {
    pub page: PageId,
    pub block: Option<BlockId>,
    #[serde(default)]
    pub date_mark: Option<DateMarkRef>,
    pub title: String,
    pub kind: AgendaKind,
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    pub has_time: bool,
    pub done: bool,
    /// A database status option's label or a task note's `todo`/`doing`/`done`.
    /// Absent for block and database checkbox tasks.
    #[serde(default)]
    pub status: Option<String>,
    /// `todo`, `progress` or `complete`, resolved by the status property's groups.
    #[serde(default)]
    pub status_group: Option<String>,
    /// The status option's own colour name, when set.
    #[serde(default)]
    pub status_color: Option<String>,
    /// The task note's calendar category id; categories are stored by the add-on.
    #[serde(default)]
    pub calendar: Option<String>,
    /// The task note's `me` or `agent` assignment.
    #[serde(default)]
    pub assignee: Option<String>,
    pub collection: Option<PageId>,
    pub space: Option<PageId>,
    pub space_color: Option<String>,
    pub icon: Option<String>,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressKind {
    Checklist,
    Tasks,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub page: PageId,
    pub title: String,
    pub icon: Option<String>,
    pub kind: ProgressKind,
    pub done: u32,
    pub total: u32,
    pub space: Option<PageId>,
    pub space_color: Option<String>,
    pub updated_ms: i64,
}

/// A page's display options from its "..." menu. They live in the page
/// document, so they sync with the page. Set with `Vault::set_page_prefs`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PagePrefs {
    #[serde(default)]
    pub small_text: bool,
    #[serde(default)]
    pub full_width: bool,
    /// Locked against editing.
    #[serde(default)]
    pub locked: bool,
    /// Vertical position of the cover image in per mille: 0 shows its top,
    /// 1000 its bottom. None is the middle.
    #[serde(default)]
    pub cover_position: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub id: BlockId,
    pub kind: BlockKind,
    pub text: String,
    /// Sorted by start, then kind; ranges of the same kind never overlap.
    pub marks: Vec<Mark>,
    pub checked: bool,
    pub children: Vec<Block>,
    /// What the block points at: the page of a `LinkToPage`, the attachment
    /// of an `Image` or `File`, the URL of a `Bookmark`, or the fence info
    /// string (language and optional flags) of a `Code` block. Set with
    /// [`BlockOp::SetTarget`]. For these kinds `text` is the caption (for a
    /// `File`, its display name).
    #[serde(default)]
    pub target: Option<String>,
    /// Text or background colour of the whole block, by name:
    /// `red`, `red_background`, and so on (`default` is written as None).
    /// Set with [`BlockOp::SetColor`].
    #[serde(default)]
    pub color: Option<String>,
    /// Display width of an image, in percent of the text column (1..=100);
    /// None for the full width. Set with [`BlockOp::SetWidth`].
    #[serde(default)]
    pub width: Option<u32>,
}

// Keep variants, wire names and the exhaustive built-in test list together.
macro_rules! block_kinds {
    ($( $(#[$attr:meta])* $variant:ident => $name:literal, )*) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum BlockKind {
            $( $(#[$attr])* $variant, )*
            /// A kind supplied by a newer version or an unavailable mod.
            Unknown(String),
        }

        impl BlockKind {
            pub const BUILTINS: &'static [BlockKind] = &[$(BlockKind::$variant,)*];

            pub fn as_str(&self) -> &str {
                match self {
                    $(BlockKind::$variant => $name,)*
                    BlockKind::Unknown(name) => name,
                }
            }

            /// Preserve unrecognised names instead of silently changing their kind.
            pub fn parse(s: &str) -> BlockKind {
                match s {
                    $($name => BlockKind::$variant,)*
                    other => BlockKind::Unknown(other.to_owned()),
                }
            }
        }
    };
}

block_kinds! {
    Paragraph => "paragraph",
    Heading1 => "heading1",
    Heading2 => "heading2",
    Heading3 => "heading3",
    Bulleted => "bulleted",
    Numbered => "numbered",
    Todo => "todo",
    Quote => "quote",
    Code => "code",
    /// A GFM pipe table, kept verbatim in `text`.
    Table => "table",
    Divider => "divider",
    /// A link to another page (`target` is its id).
    LinkToPage => "linktopage",
    /// An image attachment (`target` is its id, `text` the caption).
    Image => "image",
    /// A file attachment (`target` is its id, `text` the file name).
    File => "file",
    /// A web bookmark (`target` is the URL, `text` the caption).
    Bookmark => "bookmark",
    /// An inline database: shows the collection page `target` inside the
    /// page (the collection is usually a child of the page).
    Database => "database",
    /// A toggle list item; its children show when it is open.
    Toggle => "toggle",
    /// Toggle headings: a heading whose children fold like a toggle's.
    ToggleHeading1 => "toggleheading1",
    ToggleHeading2 => "toggleheading2",
    ToggleHeading3 => "toggleheading3",
    /// A callout (`target` is its icon, an emoji; None is the default icon).
    Callout => "callout",
    /// A row of columns; its children are `Column` blocks.
    ColumnList => "columnlist",
    /// One column of a `ColumnList`; the column's blocks are its children.
    Column => "column",
    /// A table of contents of the page's headings (no text).
    TableOfContents => "toc",
    /// A reference to a span of an agent chat. `target` is the JSON payload
    /// includes `sessionId`, `firstMessageId`, `lastMessageId`, cached chat/chapter/
    /// section titles, summary and decision. Display fields are copies taken when
    /// the reference was made, and `text` repeats the summary for notes search.
    ChatRef => "chat_ref",
    /// A live reference to another note: `target` is its id. The block stores
    /// no text; the target's title is looked up when the block is drawn,
    /// exported or searched, and is empty while the target is unknown.
    Embed => "embed",
    /// A dynamic search card. `text` is the query spec string, not rich text;
    /// the block has no children and contributes no body text to search.
    #[serde(rename = "query")]
    Query => "query",
}

/// An inline mark. `start` and `end` count Unicode scalar values (Rust
/// `char`s): not bytes, not UTF-16 code units. `start < end <= text length`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Mark {
    pub start: u32,
    pub end: u32,
    pub kind: MarkKind,
}

/// Serialized with serde's default (external) tagging:
/// `"Bold"`, `"Italic"`, `"Strike"`, `"Code"`, `{"Link": "https://..."}`,
/// `{"PageLink": "<page id>"}`, `{"Query": "<query spec>"}`, `"Underline"`, `{"Color": "red"}`,
/// `{"Date": "2026-10-01"}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MarkKind {
    Bold,
    Italic,
    Strike,
    Code,
    Link(String),
    /// An inline mention of another page. Written as `[[Title]]` in markdown.
    PageLink(PageId),
    /// A dynamic search chip. The marked text is its label; the value is
    /// the query spec string, as printed by `SearchSpec::to_query_string`.
    Query(String),
    Underline,
    /// Text or background colour by name (`red`,
    /// `red_background`). One colour per range.
    Color(String),
    /// A date mention: the marked text shows the date, the value is an ISO
    /// date (`YYYY-MM-DD`) or date-time.
    Date(String),
}

impl MarkKind {
    /// The style key used in the Loro text.
    pub(crate) fn key(&self) -> &'static str {
        match self {
            MarkKind::Bold => "bold",
            MarkKind::Italic => "italic",
            MarkKind::Strike => "strike",
            MarkKind::Code => "code",
            MarkKind::Link(_) => "link",
            MarkKind::PageLink(_) => "pagelink",
            MarkKind::Query(_) => "query",
            MarkKind::Underline => "underline",
            MarkKind::Color(_) => "color",
            MarkKind::Date(_) => "date",
        }
    }
}

/// A block to insert. The caller chooses `id`, so the interface need not wait
/// for a reply before referring to the new block. Children are inserted with
/// further `Insert` ops that name this block as their parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewBlock {
    pub id: BlockId,
    pub kind: BlockKind,
    pub text: String,
    pub marks: Vec<Mark>,
    pub checked: bool,
}

/// One edit of a page's block tree. Serialized internally tagged, e.g.
/// `{"op": "SetText", "id": "b1", "text": "Hi", "marks": []}`.
///
/// `parent: None` means the top level of the page. `after: None` means "first
/// child of `parent`"; otherwise `after` must currently be a child of `parent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum BlockOp {
    Insert {
        parent: Option<BlockId>,
        after: Option<BlockId>,
        block: NewBlock,
    },
    /// Deletes the block and all its children.
    Delete {
        id: BlockId,
    },
    Move {
        id: BlockId,
        parent: Option<BlockId>,
        after: Option<BlockId>,
    },
    /// The block's full new text and marks; the core diffs it into the CRDT.
    SetText {
        id: BlockId,
        text: String,
        marks: Vec<Mark>,
    },
    SetKind {
        id: BlockId,
        kind: BlockKind,
    },
    SetChecked {
        id: BlockId,
        checked: bool,
    },
    /// Sets what a `LinkToPage`, `Image`, `File` or `Bookmark` block points
    /// at (see [`Block::target`]). Insert the block, then set its target in
    /// the same batch.
    SetTarget {
        id: BlockId,
        target: Option<String>,
    },
    /// Sets the block's colour (see [`Block::color`]).
    SetColor {
        id: BlockId,
        color: Option<String>,
    },
    /// Sets an image's display width (see [`Block::width`]); None, 0 or
    /// 100 and above clear it.
    SetWidth {
        id: BlockId,
        width: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub id: PageId,
    pub title: String,
    /// Plain text around the best match in the body (no highlight markup).
    pub snippet: String,
    /// Found by typo correction rather than an exact or prefix match.
    #[serde(default)]
    pub fuzzy: bool,
    /// The query with its misspelled words replaced (fuzzy hits only). Title
    /// hits give the title's words; body hits give the chosen corrections.
    #[serde(default)]
    pub corrected: Option<String>,
    /// Actual values that satisfied the query's filter operators.
    #[serde(default)]
    pub matched_filters: Vec<MatchedFilterValue>,
    /// `"vault"` for pages, or the external source that owns the row.
    #[serde(default = "default_search_source")]
    pub source: String,
    /// The page type of a vault page, or the external row's type.
    #[serde(default = "default_page_type")]
    pub page_type: String,
    /// The external row's opaque target JSON (never read by the core).
    #[serde(default)]
    pub target: Option<String>,
}

/// The source of vault pages in [`SearchHit::source`].
pub const VAULT_SOURCE: &str = "vault";

pub(crate) fn default_search_source() -> String {
    VAULT_SOURCE.to_string()
}

/// A row an external source (for example agent chats) asks the index to
/// hold and search. It is never a page: no listing, tree or mutation sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalSearchRow {
    /// The source's own id for the row, unique within its source.
    pub id: String,
    pub title: String,
    /// Searched text (plain).
    pub body: String,
    /// Shown and filtered as a page type (for example `chat`).
    pub page_type: String,
    /// Opaque JSON the source uses to open the row. Returned, not read.
    pub target: Option<String>,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchedFilterValue {
    pub key: String,
    pub value: String,
}

/// Search totals saturate at 201, displayed as "200+".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchSpecResults {
    pub hits: Vec<SearchHit>,
    pub total_bounded: u32,
    pub narrowed: bool,
    pub warnings: Vec<String>,
}

/// Search rows are unique within their source, not across sources.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HitKey {
    pub source: String,
    pub id: String,
}

impl From<&SearchHit> for HitKey {
    fn from(hit: &SearchHit) -> Self {
        Self {
            source: hit.source.clone(),
            id: hit.id.clone(),
        }
    }
}

/// An initial live-query snapshot, or changes since its previous update.
/// Positions in `moved` are zero-based positions in the resulting window.
/// Added hits follow result order. `moved` also supplies each added hit's
/// position. Apply removals and additions, then place moved keys at their
/// final positions simultaneously, then replace changed hits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveQueryUpdate {
    pub generation: u64,
    pub initial: bool,
    pub added: Vec<SearchHit>,
    pub removed: Vec<HitKey>,
    pub moved: Vec<(HitKey, u32)>,
    pub changed: Vec<SearchHit>,
    pub total_bounded: u32,
}

/// Replaces the byte range [start, end) at the completion cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchCompletion {
    pub value: String,
    pub label: String,
    pub start: u32,
    pub end: u32,
    pub resolved_id: Option<String>,
}

/// Size of the typo-correction word index, for diagnostics
/// (`Vault::search_index_stats`). Zero until the first build.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SearchIndexStats {
    /// Distinct indexed terms (title and body words).
    pub words: u32,
    /// Hashed deletion forms of those terms (one or two deletions each).
    pub deletion_keys: u32,
    /// Retained word strings, character arrays and deletion pairs, by
    /// allocated capacity. Excludes titles, SQLite and allocator overhead.
    pub estimated_bytes: u64,
    /// Wall time of the last rebuild, in milliseconds.
    pub build_ms: f64,
}

/// Keystroke search results and whether breadth limited matching to titles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub narrowed: bool,
}

// ------------------------------------------------------------------ properties

/// A property value. `CreatedTime` and `UpdatedTime` are not values: they
/// come from the page (`created_ms`, `updated_ms`).
///
/// Setting `Empty`, an empty text or URL, or an empty list removes the
/// property. Numbers must be finite.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PropValue {
    Empty,
    Text(String),
    Number(f64),
    /// A date or date range. `has_time: false` means whole days (the times
    /// are then midnight UTC of the first and last day).
    Date {
        start_ms: i64,
        end_ms: Option<i64>,
        has_time: bool,
    },
    Checkbox(bool),
    /// An option id of the schema's property (on a page outside a
    /// collection, the option's name).
    Select(String),
    MultiSelect(Vec<String>),
    Status(String),
    Relation(Vec<PageId>),
    Url(String),
    /// Attachments (files and media), in display order.
    Files(Vec<AttachmentId>),
    /// A formula or rollup that could not be computed: the message. Only
    /// ever a computed value; it cannot be set.
    Error(String),
}

// Numbers are finite (NaN is rejected), so equality is reflexive.
impl Eq for PropValue {}

impl PropValue {
    /// The kind this value belongs to (`None` for `Empty`).
    pub fn kind(&self) -> Option<PropKind> {
        Some(match self {
            PropValue::Empty => return None,
            PropValue::Text(_) => PropKind::Text,
            PropValue::Number(_) => PropKind::Number,
            PropValue::Date { .. } => PropKind::Date,
            PropValue::Checkbox(_) => PropKind::Checkbox,
            PropValue::Select(_) => PropKind::Select,
            PropValue::MultiSelect(_) => PropKind::MultiSelect,
            PropValue::Status(_) => PropKind::Status,
            PropValue::Relation(_) => PropKind::Relation,
            PropValue::Url(_) => PropKind::Url,
            PropValue::Files(_) => PropKind::Files,
            PropValue::Error(_) => return None,
        })
    }
}

/// One stored property of a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Property {
    pub key: String,
    pub value: PropValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// An email address, stored as `PropValue::Text`.
    Email,
    /// A phone number, stored as `PropValue::Text`.
    Phone,
    /// Computed from the page's `created_ms`; never stored.
    CreatedTime,
    /// Computed from the page's `updated_ms`; never stored.
    UpdatedTime,
    /// Attachments, stored as `PropValue::Files`.
    Files,
    /// Computed from the row's other properties by an expression (the
    /// `formula` key of the def's config JSON); never stored in the page.
    /// The result is a `Text`, `Number`, `Checkbox`, `Date` or `Error`.
    Formula,
    /// Computed from the pages a relation of the row points at (config keys
    /// `rollupRelation`, `rollupProperty`, `rollupFunction`); never stored.
    Rollup,
}

/// An option of a select, multi-select or status property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectOption {
    /// Stable id, stored in the values. Empty in `set_schema` = assign one.
    pub id: String,
    pub name: String,
    /// A colour name chosen by the interface (for example `"green"`).
    pub color: String,
}

/// A property of a collection's schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropDef {
    /// Stable key used in the rows' values. Empty in `set_schema` = assign one.
    pub key: String,
    /// Display name; renaming changes only this.
    pub name: String,
    pub kind: PropKind,
    /// For `Select`, `MultiSelect` and `Status`, in display order.
    pub options: Vec<SelectOption>,
    /// Settings owned by the interface (for example a number format, status
    /// groups or a relation's collection), usually JSON. The core stores
    /// and syncs it, and reads status `groups` and the keys of computed kinds: `formula`
    /// (Formula), `rollupRelation`, `rollupProperty` and `rollupFunction`
    /// (Rollup). Empty when unset.
    #[serde(default)]
    pub config: String,
}

// ------------------------------------------------------------ views and queries

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ViewKind {
    Table,
    List,
    Board,
    Calendar,
    Gallery,
    Timeline,
}

/// A saved view of a collection, stored in the collection page's document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewDef {
    /// Stable id. Empty in `add_view` = assign one.
    pub id: String,
    pub name: String,
    pub kind: ViewKind,
    pub filter: Option<Filter>,
    pub sorts: Vec<Sort>,
    /// Property key the board groups by.
    pub group_by: Option<String>,
    /// Date property key of a calendar or timeline.
    pub date_prop: Option<String>,
    /// Property keys shown, in order.
    pub visible_props: Vec<String>,
    /// Gallery cards: `"$cover"` (the page cover) or an attachment property key.
    pub card_cover: Option<String>,
    /// Settings owned by the interface (column widths, card size, hidden
    /// groups and so on), usually JSON. Stored and synced, not read by the
    /// core. Empty when unset.
    #[serde(default)]
    pub config: String,
}

/// A filter tree. An empty `And` matches everything, an empty `Or` nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Filter {
    And(Vec<Filter>),
    Or(Vec<Filter>),
    Cond(Condition),
}

/// `prop` is a property key, or one of the built-ins `$title`, `$created`
/// and `$updated`, or `*` (vault scope: any property). The variant of `value`
/// says how to compare: `Text` compares text (case-insensitively), `Select` /
/// `MultiSelect` / `Relation` compare option or page ids, `Number` numbers,
/// `Date` dates. Ops that need no value (`IsEmpty`, `Checked`, ...) ignore it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    pub prop: String,
    pub op: FilterOp,
    pub value: PropValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FilterOp {
    Is,
    IsNot,
    /// Text: substring. Select, multi-select, relation: has every given id.
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
    /// Date starts before `value.start_ms`.
    Before,
    /// Date starts after `value.start_ms`.
    After,
    OnOrBefore,
    OnOrAfter,
    /// Date (or range) overlaps `[value.start_ms, value.end_ms)`; no end = open.
    WithinRange,
    Checked,
    Unchecked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sort {
    /// A property key or `$title`, `$created`, `$updated`.
    pub prop: String,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryResult {
    pub rows: Vec<PageSummary>,
    /// Matching rows in total (ignoring offset and limit).
    pub total: u32,
}

/// One board column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// The value of the column; `Empty` for "no value". Use it in a
    /// `Condition { op: Is }` (or `IsEmpty`) to query the column's rows.
    pub value: PropValue,
    /// Option name or displayed value; empty for the "no value" column.
    pub label: String,
    /// Option colour, for select and status columns.
    pub color: Option<String>,
    pub count: u32,
}

/// A page on a calendar or timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RangeEntry {
    pub page: PageSummary,
    /// The date property this entry comes from.
    pub prop: String,
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    pub has_time: bool,
}

// ------------------------------------------------------------- trash, files, history

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashedPage {
    pub page: PageSummary,
    pub trashed_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: AttachmentId,
    pub name: String,
    pub mime: String,
    /// Bytes.
    pub size: i64,
}

/// A group of changes to a page: a new version starts after 5 idle minutes
/// or when another device made the change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// Opaque; pass it to `load_page_at` / `restore_version`.
    pub id: String,
    /// Time of the version's last change (second precision).
    pub time_ms: i64,
    /// Device id that made the changes (see `Vault::device_id`).
    pub device: String,
    /// For example `"Edited content, properties"`.
    pub summary: String,
    /// Number of Loro changes in the version.
    pub changes: u32,
}

/// One entry of the change feed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Change {
    /// A successful index commit. Empty `pages` invalidates every search
    /// (for example when external-source rows or the entire index changed).
    IndexCommitted { generation: u64, pages: Vec<PageId> },
    /// Content, title, properties, icon, cover, favorite or schema changed.
    PageChanged(PageId),
    /// Created, duplicated, imported or restored from the trash.
    PageCreated(PageId),
    /// Moved to the trash or purged.
    PageDeleted(PageId),
    /// The page tree changed (create, move, trash, restore, purge).
    TreeChanged,
}

/// The result of `poll_changes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeBatch {
    /// Pass this to the next `poll_changes`.
    pub seq: u32,
    pub changes: Vec<Change>,
    /// The feed cannot say what changed since `since_seq` (too old, or from
    /// before the vault was opened): reload everything.
    pub reset: bool,
}

/// A folder on this device shown in the vault (`mounts.rs`): its `.md`
/// files are the child pages of `page`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    /// The page the folder's files are children of.
    pub page: PageId,
    /// The folder's absolute path.
    pub folder: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    /// Pages created (one per markdown file and per folder without one).
    pub pages: u32,
    pub attachments: u32,
    /// Page links whose target was not found (their original target is kept).
    pub unresolved_links: u32,
    /// The imported pages directly under the import's parent.
    pub top_pages: Vec<PageId>,
    #[serde(default)]
    pub created: u32,
    #[serde(default)]
    pub updated: u32,
    #[serde(default)]
    pub unchanged: u32,
    /// Previously imported pages with no selected source; retained in the vault.
    #[serde(default)]
    pub orphaned: u32,
    #[serde(default)]
    pub tables: u32,
    #[serde(default)]
    pub images: u32,
    #[serde(default)]
    pub resolved_links: u32,
    #[serde(default)]
    pub unresolved_targets: Vec<ImportUnresolvedTarget>,
    #[serde(default)]
    pub orphaned_sources: Vec<String>,
    #[serde(default)]
    pub skipped_files: Vec<ImportSkippedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportUnresolvedTarget {
    pub source_file: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSkippedFile {
    /// Relative source path; skipped directories end in `/`.
    pub source_file: String,
    pub reason: String,
}

// ------------------------------------------------------------------- sync

/// A page's Loro version vector, encoded (see `Vault::page_versions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageVersion {
    pub page: PageId,
    pub version: Vec<u8>,
}

/// Loro updates of one page, from `Vault::export_updates`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageUpdate {
    pub page: PageId,
    pub bytes: Vec<u8>,
}

/// What `Vault::import_updates_batch` did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncImport {
    /// Pages that were new here.
    pub created: u32,
    /// Existing pages that received changes.
    pub changed: u32,
    /// Conflicts recorded by this import.
    pub conflicts: u32,
    /// Updates skipped because the page was purged.
    pub skipped: u32,
}

/// Content that two devices changed concurrently (found when a sync import
/// brings in a change to a block that also changed here since the last
/// common version). The page keeps Loro's merge of both until the conflict
/// is resolved with `Vault::resolve_conflict`.
///
/// Stored as format 2 in the page document; records written before format 2
/// read as `Block` scope with `Present` or `Deleted` sides and no frontiers.
/// Stable id of a conflict record. Distinct for every recording, so resolved
/// log records of the same block never overwrite each other. A format 1
/// record's id is its block id.
pub type ConflictId = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub page: PageId,
    /// The record's stable id (the document key).
    pub id: ConflictId,
    /// Record format: 1 for records written before format 2 (no versions,
    /// not undoable), 2 for the current format.
    pub format: u32,
    /// The scope's anchor block: the block of a `Block` scope, the root of a
    /// `Subtree`, empty for `Page`. Kept for callers of the first format.
    pub block: BlockId,
    /// When the conflict was recorded (milliseconds).
    pub time_ms: i64,
    pub scope: ConflictScope,
    /// Side 0 is the version of the device that recorded the conflict,
    /// side 1 the incoming one.
    pub sides: Vec<ConflictSide>,
    pub status: ConflictStatus,
    /// Every resolution applied to this record, oldest first, whatever its
    /// status: a rule-applied conflict awaiting OK is `Pending` with its
    /// application here. Empty for format 1 records.
    #[serde(default)]
    pub applied: Vec<AppliedResolution>,
}

impl Conflict {
    /// Whether an applied resolution can be undone. Only format 2 records
    /// carry the frontiers that undo needs; format 1 records never become
    /// undoable.
    pub fn undoable(&self) -> bool {
        self.format >= 2 && self.applied.last().is_some_and(|a| a.undone_at.is_none())
    }

    /// Fills each side's legacy `block` view from its state. Only `Block`
    /// scope has one block per side; other scopes leave it `None`.
    pub fn with_legacy_blocks(mut self) -> Self {
        let block_scope = matches!(self.scope, ConflictScope::Block { .. });
        for side in &mut self.sides {
            side.block = match &side.state {
                SideState::Present { content } if block_scope => content.first().cloned(),
                _ => None,
            };
        }
        self
    }
}

/// What a conflict covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictScope {
    /// Text or properties of one block (not its children), edited on both
    /// sides.
    Block { block: BlockId },
    /// A block with its children: one side deleted it, the other edited
    /// something inside it.
    Subtree { root: BlockId },
    /// The whole note: one side trashed or purged it, the other edited it.
    Page,
}

impl ConflictScope {
    /// The anchor block id (empty for `Page`).
    pub fn anchor_id(&self) -> &str {
        match self {
            ConflictScope::Block { block } => block.as_str(),
            ConflictScope::Subtree { root } => root.as_str(),
            ConflictScope::Page => "",
        }
    }

    /// The scope's name, as the index mirror stores it.
    pub fn tag(&self) -> &'static str {
        match self {
            ConflictScope::Block { .. } => "block",
            ConflictScope::Subtree { .. } => "subtree",
            ConflictScope::Page => "page",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConflictSide {
    /// Device id of the change (or the Loro peer id if unknown).
    pub device: String,
    pub time_ms: i64,
    /// The page's encoded frontiers on this side (`Frontiers::encode`), as
    /// `PageDoc::version` gives them. `None` for records written before
    /// format 2: such a side cannot be rebuilt, so nothing can be undone.
    pub frontiers: Option<Vec<u8>>,
    pub state: SideState,
    /// Where the scope sat on this side, so a restore lands in place
    /// (subtree and block scope).
    pub anchor: Option<Anchor>,
    /// Legacy view: the block of a present `Block` scope side, without
    /// children; `None` otherwise. Derived from `state` by
    /// `Conflict::with_legacy_blocks`, never stored.
    #[serde(skip)]
    pub block: Option<Block>,
}

impl ConflictSide {
    /// A side with no legacy view (see `Conflict::with_legacy_blocks`).
    pub fn new(
        device: String,
        time_ms: i64,
        frontiers: Option<Vec<u8>>,
        state: SideState,
        anchor: Option<Anchor>,
    ) -> Self {
        ConflictSide {
            device,
            time_ms,
            frontiers,
            state,
            anchor,
            block: None,
        }
    }
}

/// What one side holds for the scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SideState {
    /// The scoped blocks as they were on this side, with their children
    /// (materialized for display; the frontiers are the source of truth).
    Present { content: Vec<Block> },
    /// The scope's blocks were deleted on this side.
    Deleted,
    /// The page was in Trash since `at` (milliseconds). Page scope only.
    Trashed { at: i64 },
    /// The page was permanently deleted. `tombstone_version` is the encoded
    /// version vector at purge time, as the purge records store it, so new
    /// edits can be told apart from a stale copy. Page scope only.
    Purged { tombstone_version: Vec<u8> },
}

/// Where a scope sat on one side: its parent block and previous sibling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    /// The parent block; `None` for the page root.
    pub parent: Option<BlockId>,
    /// The previous sibling; `None` for the first child.
    pub previous: Option<BlockId>,
}

/// Whether a conflict is open or was resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictStatus {
    /// Open: listed until it is resolved.
    Pending,
    /// Resolved. Carries the latest application, which is also the last
    /// entry of `Conflict::applied`; consumed undo history is retained there.
    Resolved(AppliedResolution),
}

/// How a resolution was applied, for undo and the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedResolution {
    /// When it was applied (milliseconds). The 30-day log retention counts
    /// from here, not from detection.
    pub time_ms: i64,
    /// Time this application was undone. Its before-version remains in history;
    /// a consumed application cannot be undone again.
    #[serde(default)]
    pub undone_at: Option<i64>,
    pub by: ResolutionBy,
    /// The page's encoded frontiers just before the resolution was applied.
    /// Undo restores the scope to this version. Empty only when
    /// `before_state` is `Purged`: a purged page has no frontiers here, and
    /// none are made up from the tombstone.
    #[serde(default)]
    pub before: Vec<u8>,
    /// The page's state just before, when it was not an ordinary live page.
    /// `None` means live, with `before` set.
    #[serde(default)]
    pub before_state: Option<BeforeState>,
    /// True when no version could rebuild a missing ancestor, so the
    /// scoped content was appended at the page root.
    #[serde(default)]
    pub fallback: bool,
    /// Ancestor blocks this resolution restored, in their original order.
    /// Undo removes exactly these.
    #[serde(default)]
    pub restored_ancestors: Vec<BlockId>,
    /// What was applied: `Keep`, or `UseSide` with the index of the side whose
    /// scope was restored. Automatic rules record the side they applied.
    /// `None` on records written before this field existed.
    #[serde(default)]
    pub resolution: Option<ConflictResolution>,
}

/// The page's state before a resolution, when it was not live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BeforeState {
    /// The page was in Trash since `at` (milliseconds); `before` holds its
    /// frontiers.
    Trashed { at: i64 },
    /// The page was permanently deleted here. There is no page frontier:
    /// `before` is empty. `tombstone_version` is the encoded version vector
    /// of the purge.
    Purged { tombstone_version: Vec<u8> },
}

/// Who applied a resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolutionBy {
    User,
    /// Applied by the named automatic rule.
    Auto {
        rule: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictResolution {
    /// Confirm the current scope without editing its content.
    Keep,
    /// Restore the scope as it was on this side (`Conflict::sides` index).
    UseSide(u32),
}

#[cfg(test)]
mod organizer_tests {
    use super::*;
    fn status() -> PropDef {
        PropDef {
            key: "status".into(),
            name: "Status".into(),
            kind: PropKind::Status,
            options: ["a", "b", "c"]
                .into_iter()
                .map(|id| SelectOption {
                    id: id.into(),
                    name: id.into(),
                    color: String::new(),
                })
                .collect(),
            config: String::new(),
        }
    }
    #[test]
    fn status_fallback_and_first_option() {
        let def = status();
        assert_eq!(status_group(&def, "a"), StatusGroup::Todo);
        assert_eq!(status_group(&def, "b"), StatusGroup::InProgress);
        assert_eq!(status_group(&def, "c"), StatusGroup::Complete);
        assert_eq!(status_group(&def, "unknown"), StatusGroup::Todo);
        assert_eq!(
            first_option_in(&def, StatusGroup::Complete).unwrap().id,
            "c"
        );
        let empty = PropDef {
            options: vec![],
            ..def
        };
        assert_eq!(status_group(&empty, "unknown"), StatusGroup::Todo);
        assert!(first_option_in(&empty, StatusGroup::Complete).is_none());
    }
    #[test]
    fn configured_groups_override_order() {
        let def = PropDef {
            config: r#"{"groups":{"a":"complete","b":"todo","c":"inProgress"}}"#.into(),
            ..status()
        };
        assert_eq!(status_group(&def, "a"), StatusGroup::Complete);
        assert_eq!(status_group(&def, "c"), StatusGroup::InProgress);
        assert_eq!(
            first_option_in(&def, StatusGroup::Complete).unwrap().id,
            "a"
        );
        assert_eq!(first_option_in(&def, StatusGroup::Todo).unwrap().id, "b");
    }
}

#[cfg(test)]
mod chat_ref_tests {
    use super::*;

    #[test]
    fn chat_ref_round_trips_target_json_and_summary() {
        let target = concat!(
            r#"{"sessionId":"e5445cc0-bf53-4854-8b45-690a8e61bbcc","#,
            r#""firstMessageId":"m-first","lastMessageId":"m-last","#,
            r#""title":"Replay bubbleplay","summary":"Cursor fix, driver route dropped."}"#,
        );
        let summary = "Cursor fix, driver route dropped.";
        let block = Block {
            id: "chat-block-1".into(),
            kind: BlockKind::ChatRef,
            text: summary.into(),
            marks: Vec::new(),
            checked: false,
            children: Vec::new(),
            target: Some(target.into()),
            color: None,
            width: None,
        };
        assert_eq!(BlockKind::ChatRef.as_str(), "chat_ref");
        assert_eq!(BlockKind::parse("chat_ref"), BlockKind::ChatRef);

        let json = serde_json::to_string(&block).unwrap();
        let back: Block = serde_json::from_str(&json).unwrap();
        assert_eq!(back, block);
        assert_eq!(back.kind, BlockKind::ChatRef);
        assert_eq!(back.target.as_deref(), Some(target));
        assert_eq!(back.text, summary);
        let payload: serde_json::Value =
            serde_json::from_str(back.target.as_deref().unwrap()).unwrap();
        assert_eq!(payload["sessionId"], "e5445cc0-bf53-4854-8b45-690a8e61bbcc");
        assert_eq!(payload["firstMessageId"], "m-first");
        assert_eq!(payload["lastMessageId"], "m-last");
        assert_eq!(payload["title"], "Replay bubbleplay");
        assert_eq!(payload["summary"], summary);
        assert!(payload.get("id").is_none(), "the session is sessionId");
    }
}
