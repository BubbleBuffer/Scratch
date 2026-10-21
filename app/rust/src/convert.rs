//! Conversions between the core's types and the bridge types in
//! `api::types`. Kept outside `api` so flutter_rust_bridge does not expose
//! them to Dart.

use vault_core as vc;

use crate::api::types::*;

pub(crate) fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

pub(crate) fn type_data_list_item(item: TypeDataListItem) -> Result<vc::ListItem, String> {
    match (item.index, item.id) {
        (Some(index), None) => Ok(vc::ListItem::Index(index)),
        (None, Some(id)) => Ok(vc::ListItem::Id(id)),
        _ => Err("a list item requires exactly one of index or id".into()),
    }
}

// ------------------------------------------------------------ block kinds

pub(crate) fn kind_name(k: &vc::BlockKind) -> &str {
    k.as_str()
}

pub(crate) fn kind_parse(s: &str) -> Result<vc::BlockKind, String> {
    Ok(vc::BlockKind::parse(s))
}

// ------------------------------------------------------------------ marks

/// Map from scalar offset to UTF-16 offset (index = scalar offset).
fn utf16_offsets(text: &str) -> Vec<u32> {
    let mut v = Vec::with_capacity(text.len() + 1);
    let mut u = 0u32;
    for c in text.chars() {
        v.push(u);
        u += c.len_utf16() as u32;
    }
    v.push(u);
    v
}

pub(crate) fn marks_from(text: &str, marks: Vec<vc::Mark>) -> Vec<BMark> {
    if marks.is_empty() {
        return Vec::new();
    }
    let map = utf16_offsets(text);
    let last = *map.last().unwrap_or(&0);
    let at = |i: u32| map.get(i as usize).copied().unwrap_or(last);
    marks
        .into_iter()
        .map(|m| {
            let (kind, url) = match m.kind {
                vc::MarkKind::Bold => (BMarkKind::Bold, String::new()),
                vc::MarkKind::Italic => (BMarkKind::Italic, String::new()),
                vc::MarkKind::Strike => (BMarkKind::Strike, String::new()),
                vc::MarkKind::Code => (BMarkKind::Code, String::new()),
                vc::MarkKind::Link(u) => (BMarkKind::Link, u),
                vc::MarkKind::PageLink(id) => (BMarkKind::PageLink, id),
                vc::MarkKind::Query(spec) => (BMarkKind::Query, spec),
                vc::MarkKind::Underline => (BMarkKind::Underline, String::new()),
                vc::MarkKind::Color(c) => (BMarkKind::Color, c),
                vc::MarkKind::Date(d) => (BMarkKind::Date, d),
            };
            BMark { start: at(m.start), end: at(m.end), kind, url }
        })
        .collect()
}

/// UTF-16 offsets back to scalar offsets. Offsets inside a surrogate pair
/// round down; empty ranges are dropped.
pub(crate) fn marks_to(text: &str, marks: &[BMark]) -> Vec<vc::Mark> {
    if marks.is_empty() {
        return Vec::new();
    }
    let map = utf16_offsets(text);
    let to_scalar = |u: u32| match map.binary_search(&u) {
        Ok(i) => i as u32,
        Err(i) => i.saturating_sub(1) as u32,
    };
    marks
        .iter()
        .filter_map(|m| {
            let (s, e) = (to_scalar(m.start), to_scalar(m.end));
            if s >= e {
                return None;
            }
            let kind = match m.kind {
                BMarkKind::Bold => vc::MarkKind::Bold,
                BMarkKind::Italic => vc::MarkKind::Italic,
                BMarkKind::Strike => vc::MarkKind::Strike,
                BMarkKind::Code => vc::MarkKind::Code,
                BMarkKind::Link => vc::MarkKind::Link(m.url.clone()),
                BMarkKind::PageLink => vc::MarkKind::PageLink(m.url.clone()),
                BMarkKind::Query => vc::MarkKind::Query(m.url.clone()),
                BMarkKind::Underline => vc::MarkKind::Underline,
                BMarkKind::Color => vc::MarkKind::Color(m.url.clone()),
                BMarkKind::Date => vc::MarkKind::Date(m.url.clone()),
            };
            Some(vc::Mark { start: s, end: e, kind })
        })
        .collect()
}

// ----------------------------------------------------------------- blocks

pub(crate) fn flat_block(b: vc::Block, depth: u32) -> (FlatBlock, Vec<vc::Block>) {
    let marks = marks_from(&b.text, b.marks);
    (
        FlatBlock {
            id: b.id,
            kind: kind_name(&b.kind).to_string(),
            text: b.text,
            marks,
            checked: b.checked,
            depth,
            target: b.target,
            color: b.color,
            width: b.width,
        },
        b.children,
    )
}

pub(crate) fn flatten(blocks: Vec<vc::Block>, depth: u32, out: &mut Vec<FlatBlock>) {
    for b in blocks {
        let (fb, children) = flat_block(b, depth);
        out.push(fb);
        flatten(children, depth + 1, out);
    }
}

pub(crate) fn page_data(p: vc::Page, load_us: u32) -> PageData {
    let mut blocks = Vec::new();
    flatten(p.blocks, 0, &mut blocks);
    PageData {
        id: p.id,
        title: p.title,
        blocks,
        parent: p.parent,
        icon: p.icon,
        cover: p.cover,
        color: p.color,
        role: p.role,
        properties: p.properties.into_iter().map(property).collect(),
        page_type: p.page_type,
        favorite: p.favorite,
        in_trash: p.in_trash,
        created_ms: p.created_ms,
        updated_ms: p.updated_ms,
        prefs: PagePrefsData {
            small_text: p.prefs.small_text,
            full_width: p.prefs.full_width,
            locked: p.prefs.locked,
            cover_position: p.prefs.cover_position,
        },
        load_us,
    }
}

pub(crate) fn page_row(p: vc::PageSummary) -> PageRow {
    PageRow {
        id: p.id,
        title: p.title,
        parent: p.parent,
        updated_ms: p.updated_ms,
        created_ms: p.created_ms,
        child_count: p.child_count,
        icon: p.icon,
        page_type: p.page_type,
        favorite: p.favorite,
        properties: p.properties.into_iter().map(property).collect(),
    }
}

pub(crate) fn rows(v: Vec<vc::PageSummary>) -> Vec<PageRow> {
    v.into_iter().map(page_row).collect()
}

pub(crate) fn agenda_settings_from(s: vc::AgendaSettings) -> AgendaSettings {
    let role = match s.role {
        vc::AgendaRole::Auto => AgendaRole::Auto,
        vc::AgendaRole::Tasks => AgendaRole::Tasks,
        vc::AgendaRole::Events => AgendaRole::Events,
        vc::AgendaRole::Off => AgendaRole::Off,
    };
    AgendaSettings {
        role,
        date_prop: s.date_prop,
    }
}

pub(crate) fn status_group_from(group: vc::StatusGroup) -> StatusGroup {
    match group {
        vc::StatusGroup::Todo => StatusGroup::Todo,
        vc::StatusGroup::InProgress => StatusGroup::InProgress,
        vc::StatusGroup::Complete => StatusGroup::Complete,
    }
}

pub(crate) fn status_group_to(group: StatusGroup) -> vc::StatusGroup {
    match group {
        StatusGroup::Todo => vc::StatusGroup::Todo,
        StatusGroup::InProgress => vc::StatusGroup::InProgress,
        StatusGroup::Complete => vc::StatusGroup::Complete,
    }
}

pub(crate) fn agenda_settings_to(s: AgendaSettings) -> vc::AgendaSettings {
    let role = match s.role {
        AgendaRole::Auto => vc::AgendaRole::Auto,
        AgendaRole::Tasks => vc::AgendaRole::Tasks,
        AgendaRole::Events => vc::AgendaRole::Events,
        AgendaRole::Off => vc::AgendaRole::Off,
    };
    vc::AgendaSettings {
        role,
        date_prop: s.date_prop,
    }
}

pub(crate) fn agenda_item(i: vc::AgendaItem) -> AgendaItem {
    AgendaItem {
        page: i.page,
        block: i.block,
        date_mark: i.date_mark.map(|m| DateMarkRef {
            start: m.start,
            end: m.end,
            iso: m.iso,
        }),
        title: i.title,
        kind: match i.kind {
            vc::AgendaKind::Task => AgendaKind::Task,
            vc::AgendaKind::Event => AgendaKind::Event,
            vc::AgendaKind::Mention => AgendaKind::Mention,
        },
        start_ms: i.start_ms,
        end_ms: i.end_ms,
        has_time: i.has_time,
        done: i.done,
        status: i.status,
        status_group: i.status_group,
        status_color: i.status_color,
        calendar: i.calendar,
        assignee: i.assignee,
        collection: i.collection,
        space: i.space,
        space_color: i.space_color,
        icon: i.icon,
        updated_ms: i.updated_ms,
    }
}

pub(crate) fn calendar_category_from(c: vc::CalendarCategory) -> CalendarCategory {
    CalendarCategory {
        id: c.id,
        name: c.name,
        color: c.color,
        show_in_calendar: c.show_in_calendar,
        show_in_today: c.show_in_today,
    }
}

pub(crate) fn calendar_category_to(c: CalendarCategory) -> vc::CalendarCategory {
    vc::CalendarCategory {
        id: c.id,
        name: c.name,
        color: c.color,
        show_in_calendar: c.show_in_calendar,
        show_in_today: c.show_in_today,
    }
}

pub(crate) fn calendar_settings_from(s: vc::CalendarSettings) -> CalendarSettings {
    CalendarSettings {
        calendars: s.calendars.into_iter().map(calendar_category_from).collect(),
        databases: s
            .databases
            .into_iter()
            .map(|d| CalendarDatabase { id: d.id, show_in_calendar: d.show_in_calendar })
            .collect(),
    }
}

pub(crate) fn progress(p: vc::Progress) -> Progress {
    Progress {
        page: p.page,
        title: p.title,
        icon: p.icon,
        kind: match p.kind {
            vc::ProgressKind::Checklist => ProgressKind::Checklist,
            vc::ProgressKind::Tasks => ProgressKind::Tasks,
        },
        done: p.done,
        total: p.total,
        space: p.space,
        space_color: p.space_color,
        updated_ms: p.updated_ms,
    }
}

pub(crate) fn block_op(o: BOp) -> Result<vc::BlockOp, String> {
    Ok(match o.op {
        OpKind::Insert => {
            let marks = marks_to(&o.text, &o.marks);
            vc::BlockOp::Insert {
                parent: o.parent,
                after: o.after,
                block: vc::NewBlock {
                    id: o.id,
                    kind: kind_parse(&o.kind)?,
                    text: o.text,
                    marks,
                    checked: o.checked,
                },
            }
        }
        OpKind::Delete => vc::BlockOp::Delete { id: o.id },
        OpKind::Move => vc::BlockOp::Move { id: o.id, parent: o.parent, after: o.after },
        OpKind::SetText => {
            let marks = marks_to(&o.text, &o.marks);
            vc::BlockOp::SetText { id: o.id, text: o.text, marks }
        }
        OpKind::SetKind => vc::BlockOp::SetKind { id: o.id, kind: kind_parse(&o.kind)? },
        OpKind::SetChecked => vc::BlockOp::SetChecked { id: o.id, checked: o.checked },
        OpKind::SetTarget => vc::BlockOp::SetTarget { id: o.id, target: o.target },
        OpKind::SetColor => vc::BlockOp::SetColor { id: o.id, color: o.color },
        OpKind::SetWidth => vc::BlockOp::SetWidth { id: o.id, width: o.width },
    })
}

// ------------------------------------------------------------- properties

pub(crate) fn prop_kind_from(k: vc::PropKind) -> PropKind {
    use vc::PropKind as K;
    match k {
        K::Text => PropKind::Text,
        K::Number => PropKind::Number,
        K::Date => PropKind::Date,
        K::Checkbox => PropKind::Checkbox,
        K::Select => PropKind::Select,
        K::MultiSelect => PropKind::MultiSelect,
        K::Status => PropKind::Status,
        K::Relation => PropKind::Relation,
        K::Url => PropKind::Url,
        K::Email => PropKind::Email,
        K::Phone => PropKind::Phone,
        K::CreatedTime => PropKind::CreatedTime,
        K::UpdatedTime => PropKind::UpdatedTime,
        K::Files => PropKind::Files,
        K::Formula => PropKind::Formula,
        K::Rollup => PropKind::Rollup,
    }
}

pub(crate) fn prop_kind_to(k: PropKind) -> vc::PropKind {
    use vc::PropKind as K;
    match k {
        PropKind::Text => K::Text,
        PropKind::Number => K::Number,
        PropKind::Date => K::Date,
        PropKind::Checkbox => K::Checkbox,
        PropKind::Select => K::Select,
        PropKind::MultiSelect => K::MultiSelect,
        PropKind::Status => K::Status,
        PropKind::Relation => K::Relation,
        PropKind::Url => K::Url,
        PropKind::Email => K::Email,
        PropKind::Phone => K::Phone,
        PropKind::CreatedTime => K::CreatedTime,
        PropKind::UpdatedTime => K::UpdatedTime,
        PropKind::Files => K::Files,
        PropKind::Formula => K::Formula,
        PropKind::Rollup => K::Rollup,
    }
}

fn blank(kind: ValueKind) -> PropValue {
    PropValue {
        kind,
        text: String::new(),
        number: 0.0,
        start_ms: 0,
        end_ms: None,
        has_time: false,
        checked: false,
        ids: Vec::new(),
    }
}

pub(crate) fn value_from(v: vc::PropValue) -> PropValue {
    use vc::PropValue as V;
    match v {
        V::Empty => blank(ValueKind::Empty),
        V::Text(t) => PropValue { text: t, ..blank(ValueKind::Text) },
        V::Number(n) => PropValue { number: n, ..blank(ValueKind::Number) },
        V::Date { start_ms, end_ms, has_time } => {
            PropValue { start_ms, end_ms, has_time, ..blank(ValueKind::Date) }
        }
        V::Checkbox(c) => PropValue { checked: c, ..blank(ValueKind::Checkbox) },
        V::Select(s) => PropValue { text: s, ..blank(ValueKind::Select) },
        V::MultiSelect(ids) => PropValue { ids, ..blank(ValueKind::MultiSelect) },
        V::Status(s) => PropValue { text: s, ..blank(ValueKind::Status) },
        V::Relation(ids) => PropValue { ids, ..blank(ValueKind::Relation) },
        V::Url(u) => PropValue { text: u, ..blank(ValueKind::Url) },
        V::Files(ids) => PropValue { ids, ..blank(ValueKind::Files) },
        V::Error(e) => PropValue { text: e, ..blank(ValueKind::Error) },
    }
}

pub(crate) fn value_to(v: PropValue) -> vc::PropValue {
    use vc::PropValue as V;
    match v.kind {
        ValueKind::Empty => V::Empty,
        ValueKind::Text => V::Text(v.text),
        ValueKind::Number => V::Number(v.number),
        ValueKind::Date => V::Date { start_ms: v.start_ms, end_ms: v.end_ms, has_time: v.has_time },
        ValueKind::Checkbox => V::Checkbox(v.checked),
        ValueKind::Select => V::Select(v.text),
        ValueKind::MultiSelect => V::MultiSelect(v.ids),
        ValueKind::Status => V::Status(v.text),
        ValueKind::Relation => V::Relation(v.ids),
        ValueKind::Url => V::Url(v.text),
        ValueKind::Files => V::Files(v.ids),
        ValueKind::Error => V::Error(v.text),
    }
}

pub(crate) fn property(p: vc::Property) -> Property {
    Property { key: p.key, value: value_from(p.value) }
}

pub(crate) fn prop_def_from(d: vc::PropDef) -> PropDef {
    PropDef {
        key: d.key,
        name: d.name,
        kind: prop_kind_from(d.kind),
        options: d
            .options
            .into_iter()
            .map(|o| SelectOption { id: o.id, name: o.name, color: o.color })
            .collect(),
        config: d.config,
    }
}

pub(crate) fn prop_def_to(d: PropDef) -> vc::PropDef {
    vc::PropDef {
        key: d.key,
        name: d.name,
        kind: prop_kind_to(d.kind),
        options: d
            .options
            .into_iter()
            .map(|o| vc::SelectOption { id: o.id, name: o.name, color: o.color })
            .collect(),
        config: d.config,
    }
}

// ------------------------------------------------------- views and queries

fn view_kind_from(k: vc::ViewKind) -> ViewKind {
    use vc::ViewKind as K;
    match k {
        K::Table => ViewKind::Table,
        K::List => ViewKind::List,
        K::Board => ViewKind::Board,
        K::Calendar => ViewKind::Calendar,
        K::Gallery => ViewKind::Gallery,
        K::Timeline => ViewKind::Timeline,
    }
}

fn view_kind_to(k: ViewKind) -> vc::ViewKind {
    use vc::ViewKind as K;
    match k {
        ViewKind::Table => K::Table,
        ViewKind::List => K::List,
        ViewKind::Board => K::Board,
        ViewKind::Calendar => K::Calendar,
        ViewKind::Gallery => K::Gallery,
        ViewKind::Timeline => K::Timeline,
    }
}

macro_rules! op_map {
    ($($v:ident),*) => {
        fn op_from(o: vc::FilterOp) -> FilterOp {
            match o { $(vc::FilterOp::$v => FilterOp::$v),* }
        }
        fn op_to(o: FilterOp) -> vc::FilterOp {
            match o { $(FilterOp::$v => vc::FilterOp::$v),* }
        }
    };
}

op_map!(
    Is, IsNot, Contains, NotContains, StartsWith, EndsWith, IsEmpty, IsNotEmpty, Greater,
    GreaterOrEqual, Less, LessOrEqual, Before, After, OnOrBefore, OnOrAfter, WithinRange, Checked,
    Unchecked
);

pub(crate) fn filter_from(f: vc::Filter) -> Filter {
    match f {
        vc::Filter::And(v) => Filter {
            kind: FilterKind::And,
            children: v.into_iter().map(filter_from).collect(),
            cond: None,
        },
        vc::Filter::Or(v) => Filter {
            kind: FilterKind::Or,
            children: v.into_iter().map(filter_from).collect(),
            cond: None,
        },
        vc::Filter::Cond(c) => Filter {
            kind: FilterKind::Cond,
            children: Vec::new(),
            cond: Some(Condition { prop: c.prop, op: op_from(c.op), value: value_from(c.value) }),
        },
    }
}

pub(crate) fn filter_to(f: Filter) -> Result<vc::Filter, String> {
    Ok(match f.kind {
        FilterKind::And => {
            vc::Filter::And(f.children.into_iter().map(filter_to).collect::<Result<_, _>>()?)
        }
        FilterKind::Or => {
            vc::Filter::Or(f.children.into_iter().map(filter_to).collect::<Result<_, _>>()?)
        }
        FilterKind::Cond => {
            let c = f.cond.ok_or("a Cond filter needs a condition")?;
            vc::Filter::Cond(vc::Condition {
                prop: c.prop,
                op: op_to(c.op),
                value: value_to(c.value),
            })
        }
    })
}

pub(crate) fn opt_filter_to(f: Option<Filter>) -> Result<Option<vc::Filter>, String> {
    f.map(filter_to).transpose()
}

// ------------------------------------------------------------ search

pub(crate) fn external_row_from(r: ExternalSearchRow) -> vc::ExternalSearchRow {
    vc::ExternalSearchRow {
        id: r.id,
        title: r.title,
        body: r.body,
        page_type: r.page_type,
        target: r.target,
        updated_ms: r.updated_ms,
    }
}

pub(crate) fn hit_from(h: vc::SearchHit) -> Hit {
    Hit {
        id: h.id,
        title: h.title,
        snippet: h.snippet,
        fuzzy: h.fuzzy,
        corrected: h.corrected,
        source: h.source,
        page_type: h.page_type,
        target: h.target,
        matched_filters: h
            .matched_filters
            .into_iter()
            .map(matched_filter_from)
            .collect(),
    }
}

fn matched_filter_from(m: vc::MatchedFilterValue) -> MatchedFilterValue {
    MatchedFilterValue {
        key: m.key,
        value: m.value,
    }
}

pub(crate) fn search_results_from(r: vc::SearchSpecResults) -> SearchSpecResults {
    SearchSpecResults {
        hits: r.hits.into_iter().map(hit_from).collect(),
        total_bounded: r.total_bounded,
        narrowed: r.narrowed,
        warnings: r.warnings,
    }
}

pub(crate) fn completion_from(c: vc::SearchCompletion) -> SearchCompletion {
    SearchCompletion {
        value: c.value,
        label: c.label,
        start: c.start,
        end: c.end,
        resolved_id: c.resolved_id,
    }
}

pub(crate) fn parsed_query_from(p: vc::ParsedQuery) -> ParsedQuery {
    ParsedQuery {
        spec: spec_from(p.spec),
        tokens: p.tokens.into_iter().map(token_from).collect(),
    }
}

pub(crate) fn spec_from(s: vc::SearchSpec) -> SearchSpec {
    SearchSpec {
        terms: s.terms.into_iter().map(term_from).collect(),
        operators: s.operators.into_iter().map(operator_from).collect(),
        sort: s.sort,
        warnings: s.warnings,
    }
}

pub(crate) fn spec_to(s: SearchSpec) -> Result<vc::SearchSpec, String> {
    Ok(vc::SearchSpec {
        terms: s.terms.into_iter().map(term_to).collect(),
        operators: s
            .operators
            .into_iter()
            .map(operator_to)
            .collect::<Result<_, _>>()?,
        sort: s.sort,
        warnings: s.warnings,
    })
}

fn term_from(t: vc::SearchTerm) -> SearchTerm {
    SearchTerm {
        value: t.value,
        phrase: t.phrase,
        exclude: t.exclude,
    }
}

fn term_to(t: SearchTerm) -> vc::SearchTerm {
    vc::SearchTerm {
        value: t.value,
        phrase: t.phrase,
        exclude: t.exclude,
    }
}

fn operator_from(o: vc::SearchOperator) -> SearchOperator {
    SearchOperator {
        key: o.key,
        value: o.value,
        negated: o.negated,
        property: o.property,
        resolved_id: o.resolved_id,
        filter: o.filter.map(filter_from),
        date_future: o.date_future,
    }
}

fn operator_to(o: SearchOperator) -> Result<vc::SearchOperator, String> {
    Ok(vc::SearchOperator {
        key: o.key,
        value: o.value,
        negated: o.negated,
        property: o.property,
        resolved_id: o.resolved_id,
        filter: opt_filter_to(o.filter)?,
        date_future: o.date_future,
    })
}

fn token_from(t: vc::Token) -> Token {
    Token {
        start: t.start,
        end: t.end,
        kind: token_kind_from(t.kind),
    }
}

fn token_kind_from(k: vc::TokenKind) -> TokenKind {
    let none = TokenKind {
        kind: SearchTokenKind::Text,
        value: String::new(),
        phrase: false,
        key: String::new(),
        resolved_id: None,
        message: String::new(),
    };
    match k {
        vc::TokenKind::Text { value } => TokenKind {
            kind: SearchTokenKind::Text,
            value,
            ..none
        },
        vc::TokenKind::Phrase { value } => TokenKind {
            kind: SearchTokenKind::Phrase,
            value,
            ..none
        },
        vc::TokenKind::Exclude { value, phrase } => TokenKind {
            kind: SearchTokenKind::Exclude,
            value,
            phrase,
            ..none
        },
        vc::TokenKind::Operator {
            key,
            value,
            resolved_id,
        } => TokenKind {
            kind: SearchTokenKind::Operator,
            key,
            value,
            resolved_id,
            ..none
        },
        vc::TokenKind::Error { message } => TokenKind {
            kind: SearchTokenKind::Error,
            message,
            ..none
        },
    }
}

pub(crate) fn sorts_to(s: Vec<SortSpec>) -> Vec<vc::Sort> {
    s.into_iter().map(|s| vc::Sort { prop: s.prop, descending: s.descending }).collect()
}

pub(crate) fn view_from(v: vc::ViewDef) -> ViewDef {
    ViewDef {
        id: v.id,
        name: v.name,
        kind: view_kind_from(v.kind),
        filter: v.filter.map(filter_from),
        sorts: v
            .sorts
            .into_iter()
            .map(|s| SortSpec { prop: s.prop, descending: s.descending })
            .collect(),
        group_by: v.group_by,
        date_prop: v.date_prop,
        visible_props: v.visible_props,
        card_cover: v.card_cover,
        config: v.config,
    }
}

pub(crate) fn view_to(v: ViewDef) -> Result<vc::ViewDef, String> {
    Ok(vc::ViewDef {
        id: v.id,
        name: v.name,
        kind: view_kind_to(v.kind),
        filter: opt_filter_to(v.filter)?,
        sorts: sorts_to(v.sorts),
        group_by: v.group_by,
        date_prop: v.date_prop,
        visible_props: v.visible_props,
        card_cover: v.card_cover,
        config: v.config,
    })
}

pub(crate) fn group(g: vc::Group) -> GroupRow {
    GroupRow { value: value_from(g.value), label: g.label, color: g.color, count: g.count }
}

pub(crate) fn range_entry(e: vc::RangeEntry) -> RangeEntry {
    RangeEntry {
        page: page_row(e.page),
        prop: e.prop,
        start_ms: e.start_ms,
        end_ms: e.end_ms,
        has_time: e.has_time,
    }
}

// ------------------------------------------------------------------ misc

pub(crate) fn change_batch(b: vc::ChangeBatch) -> ChangeBatch {
    ChangeBatch {
        seq: b.seq,
        reset: b.reset,
        changes: b
            .changes
            .into_iter()
            .map(|c| {
                let (kind, page, generation, pages) = match c {
                    vc::Change::PageChanged(p) => (ChangeKind::PageChanged, Some(p), None, vec![]),
                    vc::Change::PageCreated(p) => (ChangeKind::PageCreated, Some(p), None, vec![]),
                    vc::Change::PageDeleted(p) => (ChangeKind::PageDeleted, Some(p), None, vec![]),
                    vc::Change::TreeChanged => (ChangeKind::TreeChanged, None, None, vec![]),
                    vc::Change::IndexCommitted { generation, pages } =>
                        (ChangeKind::IndexCommitted, None, Some(generation), pages),
                };
                Change { kind, page, generation, pages }
            })
            .collect(),
    }
}

pub(crate) fn sync_import(s: vc::SyncImport) -> SyncImport {
    SyncImport { created: s.created, changed: s.changed, conflicts: s.conflicts, skipped: s.skipped }
}

pub(crate) fn conflict_scope(scope: vc::ConflictScope) -> ConflictScope {
    match scope {
        vc::ConflictScope::Block { block } => ConflictScope { kind: ConflictScopeKind::Block, block: Some(block) },
        vc::ConflictScope::Subtree { root } => ConflictScope { kind: ConflictScopeKind::Subtree, block: Some(root) },
        vc::ConflictScope::Page => ConflictScope { kind: ConflictScopeKind::Page, block: None },
    }
}

pub(crate) fn conflict_state(state: vc::SideState) -> ConflictSideState {
    let mut content = Vec::new();
    let (kind, trashed_ms, tombstone_version) = match state {
        vc::SideState::Present { content: blocks } => {
            flatten(blocks, 0, &mut content);
            (ConflictStateKind::Present, None, None)
        }
        vc::SideState::Deleted => (ConflictStateKind::Deleted, None, None),
        vc::SideState::Trashed { at } => (ConflictStateKind::Trashed, Some(at), None),
        vc::SideState::Purged { tombstone_version } => (ConflictStateKind::Purged, None, Some(tombstone_version)),
    };
    ConflictSideState { kind, content, trashed_ms, tombstone_version }
}

fn applied_resolution(applied: vc::AppliedResolution) -> AppliedResolution {
    let (by, rule) = match applied.by {
        vc::ResolutionBy::User => (ResolutionBy::User, None),
        vc::ResolutionBy::Auto { rule } => (ResolutionBy::Auto, Some(rule)),
    };
    let before_state = applied.before_state.map(|state| match state {
        vc::BeforeState::Trashed { at } => ConflictBeforeState {
            kind: ConflictStateKind::Trashed, trashed_ms: Some(at), tombstone_version: None,
        },
        vc::BeforeState::Purged { tombstone_version } => ConflictBeforeState {
            kind: ConflictStateKind::Purged, trashed_ms: None, tombstone_version: Some(tombstone_version),
        },
    });
    AppliedResolution {
        time_ms: applied.time_ms, undone_at: applied.undone_at, by, rule, before: applied.before, before_state,
        fallback: applied.fallback, restored_ancestors: applied.restored_ancestors,
        resolution: applied.resolution.map(|choice| match choice {
            vc::ConflictResolution::Keep => Resolution { kind: ResolutionKind::Keep, side: 0 },
            vc::ConflictResolution::UseSide(side) => Resolution { kind: ResolutionKind::UseSide, side },
        }),
    }
}

pub(crate) fn conflict(c: vc::Conflict, device: &str) -> Conflict {
    let undoable = c.undoable();
    let block_scope = matches!(c.scope, vc::ConflictScope::Block { .. });
    Conflict {
        id: c.id,
        format: c.format,
        page: c.page,
        block: c.block,
        time_ms: c.time_ms,
        scope: conflict_scope(c.scope),
        status: match c.status {
            vc::ConflictStatus::Pending => ConflictStatus::Pending,
            vc::ConflictStatus::Resolved(_) => ConflictStatus::Resolved,
        },
        applied: c.applied.into_iter().map(applied_resolution).collect(),
        undoable,
        sides: c
            .sides
            .into_iter()
            .map(|s| {
                let state = conflict_state(s.state);
                let block = if block_scope { state.content.first().cloned() } else { None };
                ConflictSide {
                    local: s.device == device,
                    device: s.device,
                    time_ms: s.time_ms,
                    frontiers: s.frontiers,
                    state,
                    anchor: s.anchor.map(|a| ConflictAnchor { parent: a.parent, previous: a.previous }),
                    block,
                }
            })
            .collect(),
    }
}

pub(crate) fn resolution_to(r: Resolution) -> vc::ConflictResolution {
    match r.kind {
        ResolutionKind::Keep => vc::ConflictResolution::Keep,
        ResolutionKind::UseSide => vc::ConflictResolution::UseSide(r.side),
    }
}

fn diff_change(change: vc::ConflictDiffChange) -> ConflictDiffChange {
    match change {
        vc::ConflictDiffChange::Unchanged => ConflictDiffChange::Unchanged,
        vc::ConflictDiffChange::Modified => ConflictDiffChange::Modified,
        vc::ConflictDiffChange::Added => ConflictDiffChange::Added,
        vc::ConflictDiffChange::Removed => ConflictDiffChange::Removed,
    }
}

fn flatten_diff(blocks: Vec<vc::ConflictDiffBlock>, depth: u32, out: &mut Vec<ConflictDiffBlock>) {
    for b in blocks {
        let offsets = utf16_offsets(&b.block.text);
        let last = *offsets.last().unwrap_or(&0);
        let at = |i: u32| offsets.get(i as usize).copied().unwrap_or(last);
        let spans = b.spans.into_iter().map(|span| ConflictDiffSpan {
            start: at(span.start), end: at(span.end), change: diff_change(span.change),
        }).collect();
        out.push(ConflictDiffBlock {
            block: flat_block(b.block, depth).0, change: diff_change(b.change), spans,
        });
        flatten_diff(b.children, depth + 1, out);
    }
}

pub(crate) fn conflict_diff(diff: vc::ConflictDiff) -> ConflictDiff {
    ConflictDiff {
        scope: conflict_scope(diff.scope),
        sides: diff.sides.into_iter().map(|side| {
            let mut blocks = Vec::new();
            flatten_diff(side.blocks, 0, &mut blocks);
            ConflictDiffSide { device: side.device, local: side.local, state: conflict_state(side.state), blocks }
        }).collect(),
    }
}

pub(crate) fn sync_report(r: vault_sync::SyncReport) -> SyncReport {
    SyncReport {
        full: r.full,
        rounds: r.rounds,
        created: r.created,
        changed: r.changed,
        conflicts: r.conflicts,
        pushed: r.pushed,
        purged: r.purged,
        blobs_sent: r.blobs_sent,
        bytes_down: r.bytes_down as i64,
        bytes_up: r.bytes_up as i64,
    }
}

pub(crate) fn sync_options_to(o: SyncOptions) -> vault_sync::SyncOptions {
    vault_sync::SyncOptions {
        debounce_ms: o.debounce_ms as u64,
        max_delay_ms: o.max_delay_ms as u64,
        interval_ms: o.interval_ms as u64,
        long_poll_ms: o.long_poll_ms as u64,
        prefetch_attachments: o.prefetch_attachments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conflict_block(id: &str, text: &str, children: Vec<vc::Block>) -> vc::Block {
        vc::Block {
            id: id.into(), kind: vc::BlockKind::Paragraph, text: text.into(), marks: vec![],
            checked: false, target: None, color: None, width: None, children,
        }
    }

    #[test]
    fn conflict_bridge_preserves_scoped_tree_versions_and_resolution_history() {
        let applied = vc::AppliedResolution {
            time_ms: 20, by: vc::ResolutionBy::Auto { rule: "delete_edit".into() },
            resolution: Some(vc::ConflictResolution::UseSide(0)),
            undone_at: None,
            before: vec![],
            before_state: Some(vc::BeforeState::Purged { tombstone_version: vec![9, 8] }),
            fallback: true, restored_ancestors: vec!["parent".into()],
        };
        let core = vc::Conflict {
            id: "record".into(), format: 2, page: "page".into(), block: "root".into(), time_ms: 10,
            scope: vc::ConflictScope::Subtree { root: "root".into() },
            sides: vec![
                vc::ConflictSide::new("here".into(), 11, Some(vec![1, 2]),
                    vc::SideState::Present { content: vec![conflict_block("root", "root", vec![
                        conflict_block("child", "child", vec![conflict_block("leaf", "leaf", vec![])])
                    ])] },
                    Some(vc::Anchor { parent: Some("parent".into()), previous: Some("previous".into()) })),
                vc::ConflictSide::new("there".into(), 12, Some(vec![3]), vc::SideState::Deleted, None),
            ],
            status: vc::ConflictStatus::Resolved(applied.clone()), applied: vec![applied],
        };
        let bridge = conflict(core, "here");
        assert_eq!((&bridge.id[..], bridge.format, bridge.scope.kind), ("record", 2, ConflictScopeKind::Subtree));
        assert_eq!(bridge.status, ConflictStatus::Resolved);
        assert!(bridge.undoable);
        let side = &bridge.sides[0];
        assert!(side.local);
        assert!(!bridge.sides[1].local);
        assert_eq!(side.time_ms, 11);
        assert_eq!(side.frontiers.as_deref(), Some(&[1, 2][..]));
        assert_eq!(side.state.content.iter().map(|b| (&b.id[..], b.depth)).collect::<Vec<_>>(),
            vec![("root", 0), ("child", 1), ("leaf", 2)]);
        assert!(side.block.is_none(), "legacy block overlay must not hide a subtree's children");
        assert_eq!(side.anchor.as_ref().unwrap().previous.as_deref(), Some("previous"));
        assert_eq!(bridge.sides[1].state.kind, ConflictStateKind::Deleted);
        let applied = &bridge.applied[0];
        assert_eq!(applied.undone_at, None);
        assert_eq!(applied.by, ResolutionBy::Auto);
        assert_eq!(applied.rule.as_deref(), Some("delete_edit"));
        assert_eq!(applied.resolution.as_ref().unwrap().kind, ResolutionKind::UseSide);
        assert_eq!(applied.resolution.as_ref().unwrap().side, 0);
        assert!(applied.fallback);
        assert_eq!(applied.restored_ancestors, vec!["parent"]);
        let before = applied.before_state.as_ref().unwrap();
        assert_eq!(before.kind, ConflictStateKind::Purged);
        assert_eq!(before.tombstone_version.as_deref(), Some(&[9, 8][..]));
    }

    #[test]
    fn conflict_bridge_preserves_undone_history_on_a_reopened_entry() {
        let applied = vc::AppliedResolution {
            time_ms: 20, undone_at: Some(30), by: vc::ResolutionBy::User,
            resolution: None,
            before: vec![1], before_state: None,
            fallback: false, restored_ancestors: vec![],
        };
        let bridge = conflict(vc::Conflict {
            id: "record".into(), format: 2, page: "page".into(), block: "block".into(), time_ms: 10,
            scope: vc::ConflictScope::Block { block: "block".into() },
            sides: vec![], status: vc::ConflictStatus::Pending, applied: vec![applied],
        }, "here");
        assert_eq!(bridge.status, ConflictStatus::Pending);
        assert!(!bridge.undoable, "an already consumed application cannot be undone again");
        assert_eq!(bridge.applied[0].undone_at, Some(30));
        assert_eq!(bridge.applied[0].before, vec![1]);
        assert!(bridge.applied[0].resolution.is_none());
    }

    #[test]
    fn conflict_bridge_keeps_legacy_block_display_and_trash_purge_states() {
        let core = vc::Conflict {
            id: "block".into(), format: 1, page: "page".into(), block: "block".into(), time_ms: 0,
            scope: vc::ConflictScope::Block { block: "block".into() },
            sides: vec![vc::ConflictSide::new("here".into(), 0, None,
                vc::SideState::Present { content: vec![conflict_block("block", "old", vec![])] }, None)],
            status: vc::ConflictStatus::Pending, applied: vec![],
        };
        let bridge = conflict(core, "here");
        assert!(!bridge.undoable);
        assert_eq!(bridge.sides[0].block.as_ref().unwrap().text, "old");
        let trash = conflict_state(vc::SideState::Trashed { at: 123 });
        assert_eq!((trash.kind, trash.trashed_ms), (ConflictStateKind::Trashed, Some(123)));
        let purge = conflict_state(vc::SideState::Purged { tombstone_version: vec![7] });
        assert_eq!(purge.kind, ConflictStateKind::Purged);
        assert_eq!(purge.tombstone_version, Some(vec![7]));
    }

    #[test]
    fn conflict_diff_bridge_preserves_hierarchy_and_converts_unicode_spans_to_utf16() {
        let mut out = vec![];
        flatten_diff(vec![vc::ConflictDiffBlock {
            block: conflict_block("root", "a😀b", vec![]), change: vc::ConflictDiffChange::Modified,
            spans: vec![vc::ConflictDiffSpan { start: 1, end: 2, change: vc::ConflictDiffChange::Added }],
            children: vec![vc::ConflictDiffBlock {
                block: conflict_block("child", "child", vec![]), change: vc::ConflictDiffChange::Removed,
                spans: vec![], children: vec![],
            }],
        }], 0, &mut out);
        assert_eq!(out.iter().map(|b| (&b.block.id[..], b.block.depth)).collect::<Vec<_>>(),
            vec![("root", 0), ("child", 1)]);
        assert_eq!(out[0].change, ConflictDiffChange::Modified);
        assert_eq!(out[0].spans, vec![ConflictDiffSpan { start: 1, end: 3, change: ConflictDiffChange::Added }]);
        assert_eq!(out[1].change, ConflictDiffChange::Removed);
    }

    #[test]
    fn search_spec_round_trip_preserves_compiled_constraints_and_page_ids() {
        let core = vc::SearchSpec {
            terms: vec![vc::SearchTerm {
                value: "garden".into(),
                phrase: false,
                exclude: false,
            }],
            operators: vec![vc::SearchOperator {
                key: "due".into(),
                value: "today".into(),
                negated: true,
                property: true,
                resolved_id: Some("stable-page".into()),
                date_future: false,
                filter: Some(vc::Filter::And(vec![vc::Filter::Cond(vc::Condition {
                    prop: "due".into(),
                    op: vc::FilterOp::WithinRange,
                    value: vc::PropValue::Date {
                        start_ms: 100,
                        end_ms: Some(200),
                        has_time: true,
                    },
                })])),
            }],
            sort: Some("title".into()),
            warnings: vec!["ignored bad date".into()],
        };
        assert_eq!(spec_to(spec_from(core.clone())).unwrap(), core);
    }

    #[test]
    fn parsed_tokens_keep_byte_spans_and_error_messages() {
        let parsed = vc::search_query::parse_search("café -draft created:banana");
        let expected = parsed.spec.clone();
        let bridge = parsed_query_from(parsed);
        assert_eq!((bridge.tokens[0].start, bridge.tokens[0].end), (0, 5));
        assert_eq!(bridge.tokens[1].kind.kind, SearchTokenKind::Exclude);
        assert_eq!(bridge.tokens[2].kind.kind, SearchTokenKind::Error);
        assert!(!bridge.tokens[2].kind.message.is_empty());
        assert_eq!(spec_to(bridge.spec).unwrap(), expected);
    }

    #[test]
    fn marks_round_trip_through_utf16() {
        let text = "a😀b bold";
        let vc_marks = vec![vc::Mark { start: 2, end: 3, kind: vc::MarkKind::Bold }];
        let b = marks_from(text, vc_marks.clone());
        assert_eq!((b[0].start, b[0].end), (3, 4));
        assert_eq!(marks_to(text, &b), vc_marks);
    }

    #[test]
    fn query_mark_round_trips_payload_and_utf16_range() {
        let text = "🐈 4 open #work tasks";
        let spec = "tag:work in:[[stable-page-id]] title:\"雪 & <open>\"";
        let marks = vec![vc::Mark {
            start: 2,
            end: text.chars().count() as u32,
            kind: vc::MarkKind::Query(spec.into()),
        }];
        let bridge = marks_from(text, marks.clone());
        assert_eq!(bridge.len(), 1);
        assert_eq!(bridge[0].kind, BMarkKind::Query);
        assert_eq!(bridge[0].url, spec);
        assert_eq!(bridge[0].start, 3);
        assert_eq!(bridge[0].end, text.encode_utf16().count() as u32);
        assert_eq!(marks_to(text, &bridge), marks);
    }

    #[test]
    fn calendar_settings_keep_every_field() {
        let core = vc::CalendarSettings {
            calendars: vec![vc::CalendarCategory {
                id: "cal-1".into(),
                name: "Work".into(),
                color: "sky".into(),
                show_in_calendar: true,
                show_in_today: false,
            }],
            databases: vec![vc::CalendarDatabase { id: "db-1".into(), show_in_calendar: false }],
        };
        let bridge = calendar_settings_from(core);
        assert_eq!(
            bridge.calendars,
            vec![CalendarCategory {
                id: "cal-1".into(),
                name: "Work".into(),
                color: "sky".into(),
                show_in_calendar: true,
                show_in_today: false,
            }]
        );
        assert_eq!(
            bridge.databases,
            vec![CalendarDatabase { id: "db-1".into(), show_in_calendar: false }]
        );
        assert_eq!(calendar_category_to(bridge.calendars[0].clone()).name, "Work");
    }

    #[test]
    fn every_kind_name_parses_back() {
        for k in vc::BlockKind::BUILTINS {
            assert_eq!(&kind_parse(kind_name(k)).unwrap(), k);
        }
        let unknown = vc::BlockKind::Unknown("mod:future kind".into());
        assert_eq!(kind_parse(kind_name(&unknown)).unwrap(), unknown);
    }
}
