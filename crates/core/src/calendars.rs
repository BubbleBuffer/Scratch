//! Calendar settings are ordinary, synced type data on a hidden role page.
use std::collections::HashSet;

use crate::agenda::SPACE_COLORS;
use crate::doc::PageDoc;
use crate::error::{invalid, not_found, Result};
use crate::ids::{new_id, stamp_ms};
use crate::index::Index;
use crate::types::{CalendarCategory, CalendarSettings, PageId};
use crate::vault::{lock, Vault};
use crate::ListItem;
use serde_json::{json, Value};

// Two devices initializing an empty vault must create the same document.
const SYSTEM_PAGE: &str = "000000000000000000000000CA";
const ADDON: &str = "tasks";

/// Older indexes omitted database dates when Today was Off. Repair only those
/// collections on open, without a schema/FTS rebuild or reading page documents.
pub(crate) fn refresh_legacy_calendar_rows(db: &Index) -> Result<()> {
    let ids = db
        .conn
        .prepare_cached(
            "SELECT id FROM pages WHERE page_type = 'database' AND agenda_role = 'off'",
        )?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !ids.is_empty() {
        let tx = db.write_tx()?;
        for id in ids {
            db.refresh_collection_agenda(&id)?;
        }
        tx.commit()?;
    }
    Ok(())
}

fn path(key: &str) -> Vec<String> {
    vec![key.into()]
}

fn settings(doc: &PageDoc) -> Result<CalendarSettings> {
    serde_json::from_value(addon_data(doc)).map_err(|_| invalid("invalid calendar settings"))
}

fn addon_data(doc: &PageDoc) -> Value {
    doc.type_data_value()
        .get(ADDON)
        .cloned()
        .unwrap_or_else(|| json!({}))
}

fn validate(calendar: &CalendarCategory) -> Result<()> {
    if calendar.name.trim().is_empty() {
        return Err(invalid("calendar name is empty"));
    }
    if !SPACE_COLORS.contains(&calendar.color.as_str()) {
        return Err(invalid("unknown calendar colour"));
    }
    Ok(())
}

// Concurrent list inserts retain their CRDT identity but can share a logical
// id. Keep the first, as type-data paths do, and remove duplicate identities.
fn duplicate_indices(value: &Value, key: &str) -> Vec<u32> {
    let mut seen = HashSet::new();
    value[key]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, item)| {
            item["id"]
                .as_str()
                .and_then(|id| (!seen.insert(id.to_owned())).then_some(i as u32))
        })
        .collect()
}

pub(crate) fn needs_normalization(doc: &PageDoc) -> bool {
    let value = addon_data(doc);
    ["calendars", "databases"]
        .iter()
        .any(|key| !duplicate_indices(&value, key).is_empty())
}

/// Enforce logical ids using ordinary type-data edits, including after merges.
pub(crate) fn normalize_document(doc: &mut PageDoc) -> Result<()> {
    let value = addon_data(doc);
    for key in ["calendars", "databases"] {
        for i in duplicate_indices(&value, key).into_iter().rev() {
            doc.type_data_list_delete(ADDON, &path(key), &ListItem::Index(i))?;
        }
    }
    Ok(())
}

impl Vault {
    /// Called with the calendar mutex held. Install with role and defaults
    /// already present, so the system page never appears in visible lists.
    fn calendars_page(&self) -> Result<PageId> {
        if let Some(id) = self.find_by_role("calendars".into())? {
            return Ok(id);
        }
        let db = lock(&self.inner.db);
        if db.exists(SYSTEM_PAGE)? {
            return Err(invalid(
                "calendar system page has lost its role or is in trash",
            ));
        }
        let doc = PageDoc::new_page(
            self.inner.actor_peer(),
            SYSTEM_PAGE,
            "Calendars",
            None,
            stamp_ms(),
        )?;
        doc.set_meta_str("role", Some("calendars"))?;
        doc.type_data_set(ADDON, &path("calendars"), &json!([]))?;
        doc.type_data_set(ADDON, &path("databases"), &json!([]))?;
        for (id, name, color, today) in [
            ("personal", "Personal", "apricot", true),
            ("work", "Work", "sky", true),
            ("agent", "Agent", "duvet", false),
        ] {
            doc.type_data_list_insert(
                ADDON,
                &path("calendars"),
                None,
                &json!({
                    "id": id, "name": name, "color": color,
                    "show_in_calendar": true, "show_in_today": today,
                }),
            )?;
        }
        self.inner.install(&db, doc)
    }

    fn unique_calendars(&self, page: &str) -> Result<CalendarSettings> {
        let value = self.inner.read(page, |d| addon_data(d))?;
        let calendars = duplicate_indices(&value, "calendars");
        let databases = duplicate_indices(&value, "databases");
        if calendars.is_empty() && databases.is_empty() {
            return serde_json::from_value(value).map_err(|_| invalid("invalid calendar settings"));
        }
        self.inner.edit(page, |doc| {
            // Compute again under the document lock: a sync may have just landed.
            normalize_document(doc)?;
            settings(doc)
        })
    }

    /// Lazily initializes defaults. Deleted defaults are never recreated.
    pub fn list_calendars(&self) -> Result<CalendarSettings> {
        let _guard = lock(&self.inner.calendars);
        let page = self.calendars_page()?;
        self.unique_calendars(&page)
    }

    pub fn create_calendar(
        &self,
        name: String,
        color: String,
        show_in_calendar: bool,
        show_in_today: bool,
    ) -> Result<String> {
        let category = CalendarCategory {
            id: new_id(),
            name,
            color,
            show_in_calendar,
            show_in_today,
        };
        validate(&category)?;
        let _guard = lock(&self.inner.calendars);
        let page = self.calendars_page()?;
        self.type_data_list_insert(
            page,
            ADDON.into(),
            path("calendars"),
            None,
            serde_json::to_string(&category).unwrap(),
        )?;
        Ok(category.id)
    }

    pub fn update_calendar(&self, calendar: CalendarCategory) -> Result<()> {
        validate(&calendar)?;
        let _guard = lock(&self.inner.calendars);
        let page = self.calendars_page()?;
        self.unique_calendars(&page)?;
        self.inner.edit(&page, |doc| {
            if !settings(doc)?.calendars.iter().any(|c| c.id == calendar.id) {
                return Err(not_found("calendar", &calendar.id));
            }
            // Edit fields in place so the entry keeps its CRDT identity.
            for (key, value) in [
                ("name", json!(calendar.name)),
                ("color", json!(calendar.color)),
                ("show_in_calendar", json!(calendar.show_in_calendar)),
                ("show_in_today", json!(calendar.show_in_today)),
            ] {
                doc.type_data_set(
                    ADDON,
                    &["calendars".into(), calendar.id.clone(), key.into()],
                    &value,
                )?;
            }
            Ok(())
        })
    }

    pub fn delete_calendar(&self, id: String) -> Result<()> {
        let _guard = lock(&self.inner.calendars);
        let page = self.calendars_page()?;
        self.unique_calendars(&page)?;
        self.type_data_list_delete(page, ADDON.into(), path("calendars"), ListItem::Id(id))
    }

    /// Reorders the complete list; rejects missing, unknown or repeated ids.
    pub fn reorder_calendars(&self, ids: Vec<String>) -> Result<()> {
        let _guard = lock(&self.inner.calendars);
        let page = self.calendars_page()?;
        self.unique_calendars(&page)?;
        self.inner.edit(&page, |doc| {
            let current = settings(doc)?;
            let wanted: HashSet<&String> = ids.iter().collect();
            if wanted.len() != ids.len()
                || ids.len() != current.calendars.len()
                || current.calendars.iter().any(|c| !wanted.contains(&c.id))
            {
                return Err(invalid(
                    "reorder must contain every calendar id exactly once",
                ));
            }
            let mut order: Vec<_> = current.calendars.into_iter().map(|c| c.id).collect();
            for (i, id) in ids.iter().enumerate() {
                let from = order.iter().position(|c| c == id).unwrap();
                if from != i {
                    doc.type_data_list_move(
                        ADDON,
                        &path("calendars"),
                        &ListItem::Id(id.clone()),
                        i as u32,
                    )?;
                    order.remove(from);
                    order.insert(i, id.clone());
                }
            }
            Ok(())
        })
    }

    pub fn set_database_calendar_visibility(
        &self,
        database: PageId,
        show_in_calendar: bool,
    ) -> Result<()> {
        // Validate independently of the database's Today agenda role.
        self.agenda_settings(database.clone())?;
        let _guard = lock(&self.inner.calendars);
        let page = self.calendars_page()?;
        self.unique_calendars(&page)?;
        self.inner.edit(&page, |doc| {
            if settings(doc)?.databases.iter().any(|d| d.id == database) {
                doc.type_data_set(
                    ADDON,
                    &[
                        "databases".into(),
                        database.clone(),
                        "show_in_calendar".into(),
                    ],
                    &json!(show_in_calendar),
                )
            } else {
                doc.type_data_list_insert(
                    ADDON,
                    &path("databases"),
                    None,
                    &json!({
                        "id": database, "show_in_calendar": show_in_calendar,
                    }),
                )
            }
        })
    }
}
