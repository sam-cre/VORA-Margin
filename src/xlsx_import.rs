//! Parse a spreadsheet into sheets, then let the app map columns interactively.
//!
//! We make no assumptions about the layout. `parse()` finds the real header
//! row (skipping title rows), splits headers from data, and grabs a sample
//! row for previews. `guess_mapping()` proposes a column-to-field mapping by
//! scoring header text, which the user can override on the mapping screen.

use anyhow::{bail, Context, Result};
use calamine::{open_workbook_auto, Data, Reader};
use directories::UserDirs;
use std::path::{Path, PathBuf};

pub fn default_import_path() -> PathBuf {
    UserDirs::new()
        .and_then(|u| u.download_dir().map(Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("vora-margin-export.xlsx")
}

/// One worksheet, split into a header row and the data rows beneath it.
pub struct ParsedSheet {
    pub name: String,
    pub headers: Vec<String>,
    /// First data row's values, used for the live preview on the map screen.
    pub sample: Vec<String>,
    data: Vec<Vec<Data>>,
}

impl ParsedSheet {
    pub fn num_rows(&self) -> usize {
        self.data.len()
    }

    pub fn num_cols(&self) -> usize {
        self.headers.len()
    }

    /// Cell value as a trimmed string. `None` column (skipped) yields "".
    pub fn cell(&self, row: usize, col: Option<usize>) -> String {
        match col {
            Some(c) => self
                .data
                .get(row)
                .and_then(|r| r.get(c))
                .map(cell_to_string)
                .unwrap_or_default(),
            None => String::new(),
        }
    }

    /// Index of the column whose header equals `header` (case-insensitive).
    pub fn col_for_header(&self, header: &str) -> Option<usize> {
        self.headers
            .iter()
            .position(|h| h.eq_ignore_ascii_case(header))
    }

    /// Display label for a column: its header, or "Column N" if blank.
    pub fn header_label(&self, col: usize) -> String {
        match self.headers.get(col) {
            Some(h) if !h.trim().is_empty() => h.trim().to_string(),
            _ => format!("Column {}", col + 1),
        }
    }
}

/// A field VORA cares about. Columns are mapped onto these.
/// (Platform isn't here — it's chosen once for the whole import via a picker.)
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ImportField {
    Name,
    Sku,
    Quantity,
    Cost,
    Shipping,
    Retail,
    Notes,
}

impl ImportField {
    pub const ALL: [ImportField; 7] = [
        ImportField::Name,
        ImportField::Sku,
        ImportField::Quantity,
        ImportField::Cost,
        ImportField::Shipping,
        ImportField::Retail,
        ImportField::Notes,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ImportField::Name => "Name",
            ImportField::Sku => "SKU",
            ImportField::Quantity => "Quantity",
            ImportField::Cost => "Cost",
            ImportField::Shipping => "Shipping",
            ImportField::Retail => "Retail",
            ImportField::Notes => "Notes",
        }
    }

    /// Required to do a meaningful import.
    pub fn required(&self) -> bool {
        matches!(self, ImportField::Name | ImportField::Retail)
    }

    fn index(self) -> usize {
        match self {
            ImportField::Name => 0,
            ImportField::Sku => 1,
            ImportField::Quantity => 2,
            ImportField::Cost => 3,
            ImportField::Shipping => 4,
            ImportField::Retail => 5,
            ImportField::Notes => 6,
        }
    }
}

/// Which spreadsheet column feeds each `ImportField` (None = skip).
pub struct ColumnMapping {
    map: [Option<usize>; 7],
}

impl ColumnMapping {
    fn empty() -> Self {
        Self { map: [None; 7] }
    }

    pub fn get(&self, field: ImportField) -> Option<usize> {
        self.map[field.index()]
    }

    fn set(&mut self, field: ImportField, col: Option<usize>) {
        self.map[field.index()] = col;
    }

    /// Step the column assigned to `field`. Cycles None -> 0 -> .. -> cols-1 -> None.
    pub fn cycle(&mut self, field: ImportField, num_cols: usize, forward: bool) {
        if num_cols == 0 {
            return;
        }
        let cur = self.map[field.index()];
        let next = if forward {
            match cur {
                None => Some(0),
                Some(i) if i + 1 < num_cols => Some(i + 1),
                Some(_) => None,
            }
        } else {
            match cur {
                None => Some(num_cols - 1),
                Some(0) => None,
                Some(i) => Some(i - 1),
            }
        };
        self.map[field.index()] = next;
    }
}

fn cell_to_string(c: &Data) -> String {
    match c {
        Data::String(s) => s.trim().to_string(),
        Data::Float(f) => f.to_string(),
        Data::Int(n) => n.to_string(),
        Data::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// Find the most likely header row within the first 15 rows: the row with the
/// most non-empty text cells whose *next* row contains a number (data). This
/// makes a one-cell "STRIPE" title row lose to the real column header below it.
fn find_header_row(rows: &[Vec<Data>]) -> usize {
    let limit = rows.len().min(15);
    let mut best = 0usize;
    let mut best_score = i32::MIN;
    for i in 0..limit {
        let text_cells = rows[i]
            .iter()
            .filter(|c| matches!(c, Data::String(s) if !s.trim().is_empty()))
            .count() as i32;
        let next_numeric = rows
            .get(i + 1)
            .map(|r| r.iter().any(|c| matches!(c, Data::Float(_) | Data::Int(_))))
            .unwrap_or(false);
        let score = text_cells + if next_numeric { 5 } else { 0 };
        if score > best_score {
            best_score = score;
            best = i;
        }
    }
    best
}

/// Parse every sheet that has at least one data row.
pub fn parse(path: &Path) -> Result<Vec<ParsedSheet>> {
    let mut wb =
        open_workbook_auto(path).with_context(|| format!("opening {}", path.display()))?;

    let names = wb.sheet_names().to_vec();
    if names.is_empty() {
        bail!("The spreadsheet has no sheets.");
    }

    let mut sheets = Vec::new();
    for name in &names {
        let range = wb
            .worksheet_range(name)
            .with_context(|| format!("reading sheet '{name}'"))?;
        let rows: Vec<Vec<Data>> = range.rows().map(|r| r.to_vec()).collect();
        if rows.is_empty() {
            continue;
        }

        let hdr_idx = find_header_row(&rows);
        let headers: Vec<String> = rows[hdr_idx].iter().map(cell_to_string).collect();
        let data: Vec<Vec<Data>> = rows.iter().skip(hdr_idx + 1).cloned().collect();
        if data.is_empty() {
            continue;
        }

        let sample = data
            .iter()
            .find(|r| r.iter().any(|c| !matches!(c, Data::Empty)))
            .map(|r| r.iter().map(cell_to_string).collect())
            .unwrap_or_default();

        sheets.push(ParsedSheet {
            name: name.clone(),
            headers,
            sample,
            data,
        });
    }

    if sheets.is_empty() {
        bail!("No product rows found. Make sure the sheet has a header row and rows beneath it.");
    }
    Ok(sheets)
}

/// Pick the highest-scoring column whose score is positive.
fn pick(headers_lc: &[String], score: impl Fn(&str) -> i32) -> Option<usize> {
    let mut best: Option<(usize, i32)> = None;
    for (i, h) in headers_lc.iter().enumerate() {
        let sc = score(h.trim());
        if sc > 0 && best.map(|(_, b)| sc > b).unwrap_or(true) {
            best = Some((i, sc));
        }
    }
    best.map(|(i, _)| i)
}

/// First column whose first data cell is non-empty text (a fallback for Name).
fn first_text_column(sheet: &ParsedSheet) -> Option<usize> {
    let row = sheet.data.first()?;
    row.iter()
        .position(|c| matches!(c, Data::String(s) if !s.trim().is_empty()))
}

/// Best-guess mapping from header text. The user can override any of it.
pub fn guess_mapping(sheet: &ParsedSheet) -> ColumnMapping {
    let mut m = ColumnMapping::empty();
    let lc: Vec<String> = sheet.headers.iter().map(|s| s.to_lowercase()).collect();

    m.set(
        ImportField::Name,
        pick(&lc, |s| {
            let mut sc = 0;
            if s.contains("name") {
                sc += 3;
            }
            if s.contains("product") {
                sc += 2;
            }
            if s.contains("item") || s.contains("title") || s.contains("style") {
                sc += 2;
            }
            if s.contains("description") {
                sc += 1;
            }
            // A "product cost" column shouldn't win the Name slot.
            if s.contains("cost") || s.contains("price") {
                sc -= 3;
            }
            sc
        })
        .or_else(|| first_text_column(sheet)),
    );

    m.set(
        ImportField::Cost,
        pick(&lc, |s| {
            let mut sc = 0;
            if s.contains("cost") {
                sc += 3;
            }
            if s.contains("cogs") {
                sc += 3;
            }
            if s.contains("production") || s.contains("manufactur") {
                sc += 1;
            }
            // Avoid combined / computed columns ("Cost with shipping").
            if s.contains("with") || s.contains("ship") || s.contains("total") {
                sc -= 4;
            }
            if s.contains("price") {
                sc -= 2;
            }
            sc
        }),
    );

    m.set(
        ImportField::Shipping,
        pick(&lc, |s| {
            let mut sc = 0;
            if s.contains("ship") {
                sc += 3;
            }
            if s.contains("postage") || s.contains("freight") || s.contains("delivery") {
                sc += 3;
            }
            if s.contains("tax") {
                sc += 1;
            }
            if s.contains("with") || s.contains("total") {
                sc -= 3;
            }
            sc
        }),
    );

    m.set(
        ImportField::Retail,
        pick(&lc, |s| {
            let mut sc = 0;
            if s.contains("retail") {
                sc += 3;
            }
            if s.contains("final") {
                sc += 3;
            }
            if s.contains("sell") {
                sc += 2;
            }
            if s.contains("msrp") || s.contains("rrp") {
                sc += 2;
            }
            if s.contains("list") {
                sc += 1;
            }
            if s.contains("price") {
                sc += 2;
            }
            // Not the retail price: adjusted/profit/fee/cost columns.
            if s.contains("adjust") {
                sc -= 4;
            }
            if s.contains("profit") || s.contains("margin") || s.contains("fee") {
                sc -= 5;
            }
            if s.contains("cost") {
                sc -= 3;
            }
            sc
        }),
    );

    m.set(
        ImportField::Sku,
        pick(&lc, |s| {
            let mut sc = 0;
            if s.contains("sku") {
                sc += 3;
            }
            if s.contains("barcode") || s.contains("upc") {
                sc += 2;
            }
            if s.contains("code") {
                sc += 2;
            }
            sc
        }),
    );

    m.set(
        ImportField::Quantity,
        pick(&lc, |s| {
            let mut sc = 0;
            if s.contains("quantity") || s == "qty" {
                sc += 3;
            }
            if s.contains("stock") || s.contains("units") || s.contains("inventory") {
                sc += 2;
            }
            sc
        }),
    );

    m.set(
        ImportField::Notes,
        pick(&lc, |s| {
            let mut sc = 0;
            if s.contains("note") {
                sc += 3;
            }
            if s.contains("comment") || s.contains("memo") || s.contains("remark") {
                sc += 2;
            }
            sc
        }),
    );

    m
}
