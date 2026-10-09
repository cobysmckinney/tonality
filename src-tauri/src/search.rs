//! Searching and sorting the grid: what the search field asks for, turned
//! into SQL. The interface reads what was typed (`src/search.ts`); this side
//! only sees the terms it found.

use rusqlite::types::Value;
use serde::{Deserialize, Serialize};

/// The order the grid shows photos in. Imports and Recently Deleted keep
/// their own order whatever this is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Sort {
    /// Date taken, newest first.
    #[default]
    Newest,
    Oldest,
    /// The latest import first, then by date taken within each.
    Imported,
    /// File name, A to Z.
    Name,
}

/// Where a term's text is looked for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Field {
    /// The camera, lens, film stock and file name.
    #[default]
    Any,
    /// The camera that took the photo, or the film camera of a scan.
    Camera,
    Lens,
    /// The film stock.
    Film,
    /// The file name.
    Name,
}

/// One part of a search. A photo matches it when the text is found in the
/// field, or when it was taken in the span or month; a term that asks for
/// none of these matches nothing.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Term {
    pub field: Field,
    pub text: Option<String>,
    /// The whole value must be the text, rather than contain it.
    pub exact: bool,
    /// Taken from this time (`YYYY`, `YYYY-MM`, `YYYY-MM-DD`…, compared as text)…
    pub from: Option<String>,
    /// …and before this one.
    pub until: Option<String>,
    /// Taken in this month of any year, 1 to 12.
    pub month: Option<u32>,
}

/// What the search field asks for: the photos that match every term.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Search {
    pub terms: Vec<Term>,
}

/// One value photos in view have, and how many have it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facet {
    pub value: String,
    pub count: u32,
}

/// What there is to search for in a view, offered under the search field.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facets {
    /// Cameras, the film cameras of scans among them; the most used first.
    pub cameras: Vec<Facet>,
    pub lenses: Vec<Facet>,
    pub films: Vec<Facet>,
    /// The months photos were taken in, `YYYY-MM`, the latest first.
    pub months: Vec<Facet>,
}

/// The camera's name as people say it, from the make and model of photos
/// `p`: "Canon EOS R5" whether the model repeats the make or not.
pub(crate) const CAMERA: &str = "CASE WHEN p.model IS NULL THEN p.make \
     WHEN p.make IS NULL OR lower(substr(p.model, 1, length(p.make))) = lower(p.make) THEN p.model \
     ELSE p.make || ' ' || p.model END";

fn columns(field: Field) -> &'static [&'static str] {
    match field {
        Field::Any => &[CAMERA, "p.film_camera", "p.lens", "p.film_lens", "p.film_stock", "p.file_name"],
        Field::Camera => &[CAMERA, "p.film_camera"],
        Field::Lens => &["p.lens", "p.film_lens"],
        Field::Film => &["p.film_stock"],
        Field::Name => &["p.file_name"],
    }
}

/// Text for LIKE to take literally, escaped with a backslash.
fn literal(text: &str) -> String {
    text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// The term as a condition on photos `p`, with its values pushed onto `values`
/// (it refers to them by number, so they can be shared with what comes before).
fn condition(term: &Term, values: &mut Vec<Value>) -> String {
    let mut push = |value: Value| {
        values.push(value);
        values.len()
    };
    let mut any = Vec::new();
    if let Some(text) = term.text.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
        // LIKE ignores case (for English letters), and with no wildcard it matches the whole value.
        let pattern = if term.exact { literal(text) } else { format!("%{}%", literal(text)) };
        let at = push(Value::Text(pattern));
        any.extend(columns(term.field).iter().map(|column| format!("{column} LIKE ?{at} ESCAPE '\\'")));
    }
    let mut span = Vec::new();
    if let Some(from) = &term.from {
        span.push(format!("p.taken_at >= ?{}", push(Value::Text(from.clone()))));
    }
    if let Some(until) = &term.until {
        span.push(format!("p.taken_at < ?{}", push(Value::Text(until.clone()))));
    }
    if !span.is_empty() {
        any.push(format!("({})", span.join(" AND ")));
    }
    if let Some(month) = term.month {
        any.push(format!("CAST(substr(p.taken_at, 6, 2) AS INTEGER) = ?{}", push(Value::Integer(month.into()))));
    }
    if any.is_empty() {
        return "0".to_string();
    }
    format!("({})", any.join(" OR "))
}

/// The search as a condition on photos `p` (true for an empty one).
pub(crate) fn conditions(search: &Search, values: &mut Vec<Value>) -> String {
    if search.terms.is_empty() {
        return "1".to_string();
    }
    search.terms.iter().map(|term| condition(term, values)).collect::<Vec<_>>().join(" AND ")
}
