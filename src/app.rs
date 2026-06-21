//! Application state and all keyboard-driven behavior.
//!
//! The app is a small state machine: `Mode` decides which keys do what and
//! which overlay (if any) the UI draws. Everything mutates `Store`, which is
//! autosaved to disk after each change so the user never loses work.

use crate::model::{Category, Collection, FeeLine, FeePreset, Product, Store};
use crate::textfield::TextField;
use crate::util::{money_str, money_sym, parse_money};
use crate::{presets, storage, xlsx_import};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::path::PathBuf;

/// Which screen / overlay is currently active.
#[derive(PartialEq, Clone)]
pub enum Mode {
    Browse,
    ProductForm,
    CollectionForm,
    PickPlatform,
    Compare,
    Confirm,
    Help,
    Reverse,
    ImportPath,
    /// Map spreadsheet columns to VORA fields before importing.
    ImportMap,
    PresetEditor,
    PresetForm,
    /// Pick a destination collection to move the selected product into.
    MoveProduct,
    /// Search across all products by name / SKU.
    Search,
    /// App settings (currency symbol, etc.).
    Settings,
    /// Entering a sale discount percentage.
    SaleInput,
    /// Entering a wholesale percentage of retail.
    WholesaleInput,
}

/// How products are ordered in the products pane (session only, not persisted).
#[derive(PartialEq, Clone, Copy)]
pub enum SortMode {
    Default,
    MarginDesc,
    MarginAsc,
    Name,
}

/// Which of the two browse panes has focus.
#[derive(PartialEq, Clone, Copy)]
pub enum Focus {
    Collections,
    Products,
}

/// The fields of the product form, in tab order.
#[derive(PartialEq, Clone, Copy)]
pub enum FormField {
    Name,
    Sku,
    Note,
    CostMode,
    Production,
    Quantity,
    Retail,
    Shipping,
    Platform,
}

/// `CostMode` and `Platform` are ◀▶ togglers, not text fields.
fn is_toggle(f: FormField) -> bool {
    matches!(f, FormField::CostMode | FormField::Platform)
}

fn next_field(f: FormField) -> FormField {
    use FormField::*;
    match f {
        Name => Sku,
        Sku => Note,
        Note => CostMode,
        CostMode => Production,
        Production => Quantity,
        Quantity => Retail,
        Retail => Shipping,
        Shipping => Platform,
        Platform => Name,
    }
}

fn prev_field(f: FormField) -> FormField {
    use FormField::*;
    match f {
        Name => Platform,
        Sku => Name,
        Note => Sku,
        CostMode => Note,
        Production => CostMode,
        Quantity => Production,
        Retail => Quantity,
        Shipping => Retail,
        Platform => Shipping,
    }
}

/// Buffered state while creating or editing a product.
pub struct ProductForm {
    /// `Some(id)` when editing an existing product, `None` when creating.
    pub editing: Option<u64>,
    pub name: TextField,
    pub sku: TextField,
    pub note: TextField,
    /// The cost field: per-unit when `cost_mode` is false, total-batch when true.
    pub production: TextField,
    pub quantity: TextField,
    pub retail: TextField,
    pub shipping: TextField,
    /// false = enter cost per unit; true = enter total batch cost (÷ quantity).
    pub cost_mode: bool,
    pub platform_idx: usize,
    pub field: FormField,
}

impl ProductForm {
    /// Mutable handle to the text field that currently has focus.
    /// (Toggle fields are handled separately and never reach here.)
    fn active_text(&mut self) -> &mut TextField {
        match self.field {
            FormField::Name => &mut self.name,
            FormField::Sku => &mut self.sku,
            FormField::Note => &mut self.note,
            FormField::Production => &mut self.production,
            FormField::Quantity => &mut self.quantity,
            FormField::Retail => &mut self.retail,
            FormField::Shipping => &mut self.shipping,
            FormField::CostMode | FormField::Platform => &mut self.name, // unreachable
        }
    }

    /// Parsed quantity, never less than 1.
    pub fn qty(&self) -> u32 {
        self.quantity
            .value()
            .trim()
            .parse::<u32>()
            .unwrap_or(1)
            .max(1)
    }

    /// The resolved per-unit production cost, accounting for batch mode.
    pub fn unit_cost(&self) -> Decimal {
        let entered = parse_money(self.production.value());
        if self.cost_mode {
            (entered / Decimal::from(self.qty())).round_dp(2)
        } else {
            entered
        }
    }
}

/// Buffered state while naming/renaming a collection.
pub struct CollectionForm {
    pub editing: Option<u64>,
    pub name: TextField,
}

/// State for the reverse pricing tool: solve for a retail price that hits a
/// target margin. Cost/shipping/platform are read live from the product.
pub struct ReverseForm {
    pub product_id: u64,
    pub target_margin: TextField,
}

/// State for the interactive spreadsheet-import column-mapping screen.
pub struct ImportMapState {
    pub sheets: Vec<crate::xlsx_import::ParsedSheet>,
    /// Mapping built from `sheets[0]` and applied to every sheet (matched by
    /// header text, so multi-sheet exports with identical columns just work).
    pub mapping: crate::xlsx_import::ColumnMapping,
    /// Highlighted row. Layout: `0..ImportField::ALL.len()` field rows, then the
    /// platform picker, then one editable collection-name row per sheet.
    pub cursor: usize,
    /// Index into `App::presets` — the platform applied to every imported product.
    pub platform_idx: usize,
    /// Editable target collection name, one per sheet.
    pub coll_names: Vec<TextField>,
}

/// What the import will produce, for the live preview + summary on the map screen.
pub struct ImportPreview {
    pub valid: usize,
    pub skipped: usize,
    /// First valid product and its resolved preset, for a computed preview row.
    pub first: Option<(Product, FeePreset)>,
}

/// Which field of the custom-preset form is focused.
#[derive(Clone)]
pub enum PresetFormField {
    Name,
    Note,
    LineLabel(usize),
    LinePercent(usize),
    LineFlat(usize),
}

/// One fee-line's worth of text inputs in the custom-preset form.
pub struct PresetLineForm {
    pub label: TextField,
    pub percent: TextField,
    pub flat: TextField,
}

impl PresetLineForm {
    fn blank() -> Self {
        Self {
            label: TextField::new("Fee"),
            percent: TextField::new("0"),
            flat: TextField::new("0"),
        }
    }
}

/// Buffered state while creating or editing a custom fee preset.
pub struct CustomPresetForm {
    /// `Some(name)` when editing an existing preset; `None` when creating.
    pub editing: Option<String>,
    pub name: TextField,
    pub note: TextField,
    pub lines: Vec<PresetLineForm>,
    pub field: PresetFormField,
}

/// A pending destructive action awaiting y/n confirmation.
#[allow(clippy::enum_variant_names)]
pub enum ConfirmAction {
    DeleteCollection(u64),
    DeleteProduct(u64),
    DeletePreset(String),
}

pub struct App {
    pub store: Store,
    /// Built-in presets + user customs, flattened for indexing.
    pub presets: Vec<FeePreset>,
    pub mode: Mode,
    pub focus: Focus,
    pub sort_mode: SortMode,
    pub coll_state: ListState,
    pub prod_state: ListState,
    pub pick_state: ListState,
    pub form: Option<ProductForm>,
    pub coll_form: Option<CollectionForm>,
    pub reverse: Option<ReverseForm>,
    pub import_path: Option<TextField>,
    pub import_map: Option<ImportMapState>,
    pub confirm: Option<ConfirmAction>,
    /// Mode to return to after the Confirm dialog resolves.
    pub before_confirm: Mode,
    pub preset_editor_state: ListState,
    pub preset_form: Option<CustomPresetForm>,
    /// List state for choosing a destination when moving a product.
    pub move_list_state: ListState,
    /// ID of the product currently being moved between collections.
    pub move_product_id: Option<u64>,
    /// Live search: text query and result selection.
    pub search_query: Option<TextField>,
    pub search_results: Vec<(usize, usize)>, // (coll_idx, storage_product_idx)
    pub search_result_state: ListState,
    /// Settings form: editable currency symbol.
    pub settings_field: Option<TextField>,
    /// Sale scenario: text input while entering, confirmed value after Enter.
    pub sale_field: Option<TextField>,
    /// Fake sale calculator: desired actual selling price.
    pub fake_target_field: Option<TextField>,
    /// Which field in the sale popup is active (false = discount %, true = fake target price).
    pub sale_input_focus: bool,
    pub active_sale_pct: Option<Decimal>,
    /// Wholesale view: text input while entering, confirmed value after Enter.
    pub wholesale_field: Option<TextField>,
    pub active_wholesale_pct: Option<Decimal>,
    pub status: String,
    pub error_popup: Option<String>,
    pub should_quit: bool,
}

fn next_preset_field(f: &PresetFormField, num_lines: usize) -> PresetFormField {
    match f {
        PresetFormField::Name => PresetFormField::Note,
        PresetFormField::Note => {
            if num_lines > 0 {
                PresetFormField::LineLabel(0)
            } else {
                PresetFormField::Name
            }
        }
        PresetFormField::LineLabel(i) => PresetFormField::LinePercent(*i),
        PresetFormField::LinePercent(i) => PresetFormField::LineFlat(*i),
        PresetFormField::LineFlat(i) => {
            if i + 1 < num_lines {
                PresetFormField::LineLabel(i + 1)
            } else {
                PresetFormField::Name
            }
        }
    }
}

fn prev_preset_field(f: &PresetFormField, num_lines: usize) -> PresetFormField {
    match f {
        PresetFormField::Name => {
            if num_lines > 0 {
                PresetFormField::LineFlat(num_lines - 1)
            } else {
                PresetFormField::Note
            }
        }
        PresetFormField::Note => PresetFormField::Name,
        PresetFormField::LineLabel(i) => {
            if *i == 0 {
                PresetFormField::Note
            } else {
                PresetFormField::LineFlat(i - 1)
            }
        }
        PresetFormField::LinePercent(i) => PresetFormField::LineLabel(*i),
        PresetFormField::LineFlat(i) => PresetFormField::LinePercent(*i),
    }
}

/// Move a list selection by `delta`, clamping to `[0, len)`.
fn move_sel(state: &mut ListState, len: usize, delta: i32) {
    if len == 0 {
        state.select(None);
        return;
    }
    let cur = state.selected().unwrap_or(0) as i32;
    let next = (cur + delta).clamp(0, len as i32 - 1);
    state.select(Some(next as usize));
}

impl App {
    pub fn new() -> anyhow::Result<Self> {
        let loaded = storage::load()?;
        let store = loaded.store;
        // If a corrupt store.json was recovered, tell the user on first paint.
        let startup_notice = loaded.recovered_backup.map(|backup| {
            format!(
                "Your saved data couldn't be read, so VORA-Margin started fresh.\n\n\
                 The previous file was kept (in case it can be salvaged) at:\n{}",
                backup.display()
            )
        });
        let mut presets = presets::builtin_presets();
        presets.extend(store.custom_presets.iter().cloned());

        let mut coll_state = ListState::default();
        let mut prod_state = ListState::default();
        if !store.collections.is_empty() {
            coll_state.select(Some(0));
            if !store.collections[0].products.is_empty() {
                prod_state.select(Some(0));
            }
        }

        Ok(Self {
            store,
            presets,
            mode: Mode::Browse,
            focus: Focus::Collections,
            sort_mode: SortMode::Default,
            coll_state,
            prod_state,
            pick_state: ListState::default(),
            form: None,
            coll_form: None,
            reverse: None,
            import_path: None,
            import_map: None,
            confirm: None,
            before_confirm: Mode::Browse,
            preset_editor_state: ListState::default(),
            preset_form: None,
            move_list_state: ListState::default(),
            move_product_id: None,
            search_query: None,
            search_results: Vec::new(),
            search_result_state: ListState::default(),
            settings_field: None,
            sale_field: None,
            fake_target_field: None,
            sale_input_focus: false,
            active_sale_pct: None,
            wholesale_field: None,
            active_wholesale_pct: None,
            status: String::new(),
            error_popup: startup_notice,
            should_quit: false,
        })
    }

    // ---- Read-only accessors --------------------------------------------

    pub fn current_collection(&self) -> Option<&Collection> {
        self.coll_state
            .selected()
            .and_then(|i| self.store.collections.get(i))
    }

    pub fn current_product(&self) -> Option<&Product> {
        let coll = self.current_collection()?;
        let si = self.prod_display_to_storage()?;
        coll.products.get(si)
    }

    pub fn preset_by_name(&self, name: &str) -> Option<&FeePreset> {
        self.presets.iter().find(|p| p.name == name)
    }

    /// Currency symbol from settings (default "$").
    pub fn currency(&self) -> &str {
        &self.store.settings.currency
    }

    /// Format a Decimal as money using the user's chosen currency symbol.
    pub fn money(&self, d: rust_decimal::Decimal) -> String {
        money_sym(d, self.currency())
    }

    /// Returns product storage indices in display (sort) order for the current collection.
    pub fn sorted_indices(&self) -> Vec<usize> {
        let coll = match self.current_collection() {
            Some(c) => c,
            None => return vec![],
        };
        Self::compute_sorted_indices(coll, &self.presets, self.sort_mode)
    }

    fn compute_sorted_indices(
        coll: &Collection,
        presets: &[FeePreset],
        mode: SortMode,
    ) -> Vec<usize> {
        let n = coll.products.len();
        let mut indices: Vec<usize> = (0..n).collect();
        match mode {
            SortMode::Default => {}
            SortMode::MarginDesc | SortMode::MarginAsc => {
                let margins: Vec<rust_decimal::Decimal> = (0..n)
                    .map(|i| {
                        let p = &coll.products[i];
                        presets
                            .iter()
                            .find(|pr| pr.name == p.platform)
                            .map(|pr| crate::calc::breakdown(p, pr).margin)
                            .unwrap_or(rust_decimal::Decimal::MIN)
                    })
                    .collect();
                if mode == SortMode::MarginDesc {
                    indices.sort_by(|&a, &b| margins[b].cmp(&margins[a]));
                } else {
                    indices.sort_by(|&a, &b| margins[a].cmp(&margins[b]));
                }
            }
            SortMode::Name => {
                indices.sort_by(|&a, &b| coll.products[a].name.cmp(&coll.products[b].name));
            }
        }
        indices
    }

    /// Map the currently selected display index to its storage index.
    pub fn prod_display_to_storage(&self) -> Option<usize> {
        let di = self.prod_state.selected()?;
        self.sorted_indices().get(di).copied()
    }

    /// Map a storage index to its current display index (for selection after operations).
    fn storage_to_display(&self, storage_idx: usize) -> Option<usize> {
        self.sorted_indices()
            .iter()
            .position(|&si| si == storage_idx)
    }

    // ---- Persistence -----------------------------------------------------

    fn persist(&mut self) {
        match storage::save(&self.store) {
            Ok(()) => self.status = "Saved".into(),
            Err(e) => self.status = format!("Save error: {e}"),
        }
    }

    // ---- Selection housekeeping -----------------------------------------

    /// Reset product selection when the active collection changes.
    fn sync_product_selection(&mut self) {
        let plen = self
            .coll_state
            .selected()
            .and_then(|i| self.store.collections.get(i))
            .map(|c| c.products.len())
            .unwrap_or(0);
        self.prod_state
            .select(if plen == 0 { None } else { Some(0) });
    }

    /// Keep both selections within bounds (after deletions).
    fn clamp_selections(&mut self) {
        let clen = self.store.collections.len();
        if clen == 0 {
            self.coll_state.select(None);
            self.prod_state.select(None);
            return;
        }
        let ci = self.coll_state.selected().unwrap_or(0).min(clen - 1);
        self.coll_state.select(Some(ci));
        let plen = self.store.collections[ci].products.len();
        if plen == 0 {
            self.prod_state.select(None);
        } else {
            let pi = self.prod_state.selected().unwrap_or(0).min(plen - 1);
            self.prod_state.select(Some(pi));
        }
    }

    /// Rebuild the merged preset list after custom presets change.
    fn rebuild_presets(&mut self) {
        let mut p = presets::builtin_presets();
        p.extend(self.store.custom_presets.iter().cloned());
        self.presets = p;
    }

    /// Clamp the preset-editor list selection to valid bounds.
    fn clamp_preset_editor(&mut self) {
        let len = self.store.custom_presets.len();
        if len == 0 {
            self.preset_editor_state.select(None);
        } else {
            let cur = self.preset_editor_state.selected().unwrap_or(0).min(len - 1);
            self.preset_editor_state.select(Some(cur));
        }
    }

    // ---- Top-level key dispatch -----------------------------------------

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Ctrl+C always quits, from anywhere.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        // Any key dismisses an error popup.
        if self.error_popup.take().is_some() {
            return;
        }
        match self.mode {
            Mode::Browse => self.key_browse(key),
            Mode::ProductForm => self.key_product_form(key),
            Mode::CollectionForm => self.key_collection_form(key),
            Mode::PickPlatform => self.key_pick_platform(key),
            Mode::Compare => self.key_compare(key),
            Mode::Reverse => self.key_reverse(key),
            Mode::ImportPath => self.key_import(key),
            Mode::ImportMap => self.key_import_map(key),
            Mode::Confirm => self.key_confirm(key),
            Mode::PresetEditor => self.key_preset_editor(key),
            Mode::PresetForm => self.key_preset_form(key),
            Mode::MoveProduct => self.key_move_product(key),
            Mode::Search => self.key_search(key),
            Mode::Settings => self.key_settings(key),
            Mode::SaleInput => self.key_sale_input(key),
            Mode::WholesaleInput => self.key_wholesale_input(key),
            Mode::Help => {
                if matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter
                ) {
                    self.mode = Mode::Browse;
                }
            }
        }
    }

    // ---- Browse mode -----------------------------------------------------

    fn key_browse(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Collections => Focus::Products,
                    Focus::Products => Focus::Collections,
                };
            }
            KeyCode::Left | KeyCode::Char('h') => self.focus = Focus::Collections,
            KeyCode::Right | KeyCode::Char('l') => self.focus = Focus::Products,
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Enter => self.on_enter(),
            KeyCode::Char('n') => self.on_new(),
            KeyCode::Char('e') => self.on_edit(),
            KeyCode::Char('d') => self.on_delete(),
            KeyCode::Char('c') => self.open_compare(),
            KeyCode::Char('p') => self.open_pick_platform(),
            KeyCode::Char('r') => self.open_reverse(),
            KeyCode::Char('u') => self.duplicate_product(),
            KeyCode::Char('m') => self.open_move_product(),
            KeyCode::Char('s') => self.cycle_sort(),
            KeyCode::Char('/') => self.open_search(),
            KeyCode::Char('X') => self.export_xlsx(),
            KeyCode::Char('i') => self.open_import(),
            KeyCode::Char('f') => {
                self.clamp_preset_editor();
                self.mode = Mode::PresetEditor;
            }
            KeyCode::Char('g') => self.open_settings(),
            KeyCode::Char('%') => self.open_sale_input(),
            KeyCode::Char('w') => self.open_wholesale_input(),
            KeyCode::Char('?') => self.mode = Mode::Help,
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: i32) {
        match self.focus {
            Focus::Collections => {
                move_sel(&mut self.coll_state, self.store.collections.len(), delta);
                self.sync_product_selection();
            }
            Focus::Products => {
                let len = self
                    .current_collection()
                    .map(|c| c.products.len())
                    .unwrap_or(0);
                move_sel(&mut self.prod_state, len, delta);
            }
        }
    }

    fn on_enter(&mut self) {
        match self.focus {
            Focus::Collections => {
                let has_products = self
                    .current_collection()
                    .map(|c| !c.products.is_empty())
                    .unwrap_or(false);
                if has_products {
                    self.focus = Focus::Products;
                } else {
                    self.status = "Empty collection — press n to add a product".into();
                }
            }
            Focus::Products => self.on_edit(),
        }
    }

    fn on_new(&mut self) {
        match self.focus {
            Focus::Collections => self.open_collection_form(None),
            Focus::Products => {
                if self.current_collection().is_some() {
                    self.open_product_form(None);
                } else {
                    self.status = "Create a collection first (n on the left pane)".into();
                }
            }
        }
    }

    fn on_edit(&mut self) {
        match self.focus {
            Focus::Collections => {
                let data = self.current_collection().map(|c| (c.id, c.name.clone()));
                if data.is_some() {
                    self.open_collection_form(data);
                }
            }
            Focus::Products => {
                let prod = self.current_product().cloned();
                if let Some(p) = prod {
                    self.open_product_form(Some(p));
                }
            }
        }
    }

    fn on_delete(&mut self) {
        match self.focus {
            Focus::Collections => {
                if let Some(id) = self.current_collection().map(|c| c.id) {
                    self.confirm = Some(ConfirmAction::DeleteCollection(id));
                    self.before_confirm = Mode::Browse;
                    self.mode = Mode::Confirm;
                }
            }
            Focus::Products => {
                if let Some(id) = self.current_product().map(|p| p.id) {
                    self.confirm = Some(ConfirmAction::DeleteProduct(id));
                    self.before_confirm = Mode::Browse;
                    self.mode = Mode::Confirm;
                }
            }
        }
    }

    // ---- Collection form -------------------------------------------------

    fn open_collection_form(&mut self, existing: Option<(u64, String)>) {
        let (editing, name) = match existing {
            Some((id, n)) => (Some(id), n),
            None => (None, String::new()),
        };
        self.coll_form = Some(CollectionForm {
            editing,
            name: TextField::new(&name),
        });
        self.mode = Mode::CollectionForm;
    }

    fn key_collection_form(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.coll_form = None;
                self.mode = Mode::Browse;
                self.status = "Cancelled".into();
            }
            KeyCode::Enter => self.commit_collection_form(),
            KeyCode::Left => self.with_coll_field(|t| t.left()),
            KeyCode::Right => self.with_coll_field(|t| t.right()),
            KeyCode::Home => self.with_coll_field(|t| t.home()),
            KeyCode::End => self.with_coll_field(|t| t.end()),
            KeyCode::Backspace => self.with_coll_field(|t| t.backspace()),
            KeyCode::Delete => self.with_coll_field(|t| t.delete()),
            KeyCode::Char(c) => self.with_coll_field(|t| t.insert(c)),
            _ => {}
        }
    }

    fn with_coll_field(&mut self, f: impl FnOnce(&mut TextField)) {
        if let Some(form) = self.coll_form.as_mut() {
            f(&mut form.name);
        }
    }

    fn commit_collection_form(&mut self) {
        let name = self
            .coll_form
            .as_ref()
            .map(|f| f.name.value().trim().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            self.status = "Name can't be empty".into();
            return;
        }
        let form = self.coll_form.take().unwrap();
        match form.editing {
            Some(id) => {
                if let Some(c) = self.store.collections.iter_mut().find(|c| c.id == id) {
                    c.name = name;
                }
            }
            None => {
                let id = self.store.new_id();
                self.store.collections.push(Collection {
                    id,
                    name,
                    products: Vec::new(),
                });
                self.coll_state
                    .select(Some(self.store.collections.len() - 1));
                self.prod_state.select(None);
            }
        }
        self.persist();
        self.mode = Mode::Browse;
    }

    // ---- Product form ----------------------------------------------------

    fn open_product_form(&mut self, existing: Option<Product>) {
        const DEFAULT_PLATFORM: &str = "Stripe (Online)";
        let form = match existing {
            Some(p) => {
                let idx = self
                    .presets
                    .iter()
                    .position(|x| x.name == p.platform)
                    .unwrap_or(0);
                ProductForm {
                    editing: Some(p.id),
                    name: TextField::new(&p.name),
                    sku: TextField::new(&p.sku),
                    note: TextField::new(&p.notes),
                    production: TextField::new(&money_str(p.production_cost)),
                    quantity: TextField::new(&p.quantity.to_string()),
                    retail: TextField::new(&money_str(p.retail_price)),
                    shipping: TextField::new(&money_str(p.shipping_cost)),
                    cost_mode: false,
                    platform_idx: idx,
                    field: FormField::Name,
                }
            }
            None => {
                let idx = self
                    .presets
                    .iter()
                    .position(|x| x.name == DEFAULT_PLATFORM)
                    .unwrap_or(0);
                ProductForm {
                    editing: None,
                    name: TextField::new(""),
                    sku: TextField::new(""),
                    note: TextField::new(""),
                    production: TextField::new(""),
                    quantity: TextField::new("1"),
                    retail: TextField::new(""),
                    shipping: TextField::new(""),
                    cost_mode: false,
                    platform_idx: idx,
                    field: FormField::Name,
                }
            }
        };
        self.form = Some(form);
        self.mode = Mode::ProductForm;
    }

    fn key_product_form(&mut self, key: KeyEvent) {
        let preset_count = self.presets.len();
        match key.code {
            KeyCode::Esc => {
                self.form = None;
                self.mode = Mode::Browse;
                self.status = "Cancelled".into();
            }
            KeyCode::Enter => self.commit_product_form(),
            KeyCode::Tab | KeyCode::Down => {
                if let Some(f) = self.form.as_mut() {
                    f.field = next_field(f.field);
                }
            }
            KeyCode::BackTab | KeyCode::Up => {
                if let Some(f) = self.form.as_mut() {
                    f.field = prev_field(f.field);
                }
            }
            KeyCode::Left => {
                if let Some(f) = self.form.as_mut() {
                    match f.field {
                        FormField::Platform => f.platform_idx = f.platform_idx.saturating_sub(1),
                        FormField::CostMode => f.cost_mode = false,
                        _ => f.active_text().left(),
                    }
                }
            }
            KeyCode::Right => {
                if let Some(f) = self.form.as_mut() {
                    match f.field {
                        FormField::Platform => {
                            if f.platform_idx + 1 < preset_count {
                                f.platform_idx += 1;
                            }
                        }
                        FormField::CostMode => f.cost_mode = true,
                        _ => f.active_text().right(),
                    }
                }
            }
            KeyCode::Home => self.with_prod_text(|t| t.home()),
            KeyCode::End => self.with_prod_text(|t| t.end()),
            KeyCode::Backspace => self.with_prod_text(|t| t.backspace()),
            KeyCode::Delete => self.with_prod_text(|t| t.delete()),
            KeyCode::Char(c) => self.with_prod_text(|t| t.insert(c)),
            _ => {}
        }
    }

    /// Apply an edit to the active product text field (ignored on the
    /// platform field, which isn't text).
    fn with_prod_text(&mut self, f: impl FnOnce(&mut TextField)) {
        if let Some(form) = self.form.as_mut()
            && !is_toggle(form.field)
        {
            f(form.active_text());
        }
    }

    fn commit_product_form(&mut self) {
        let name = self
            .form
            .as_ref()
            .map(|f| f.name.value().trim().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            self.status = "Product name can't be empty".into();
            return;
        }
        let form = self.form.take().unwrap();
        let platform = self
            .presets
            .get(form.platform_idx)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let production = form.unit_cost();
        let quantity = form.qty();
        let sku = form.sku.value().trim().to_string();
        let notes = form.note.value().trim().to_string();
        let retail = parse_money(form.retail.value());
        let shipping = parse_money(form.shipping.value());

        // Reserve a new id up front so we don't double-borrow the store.
        let new_id = if form.editing.is_none() {
            Some(self.store.new_id())
        } else {
            None
        };

        let mut added_index = None;
        if let Some(ci) = self.coll_state.selected()
            && let Some(coll) = self.store.collections.get_mut(ci)
        {
            match form.editing {
                Some(id) => {
                    if let Some(p) = coll.products.iter_mut().find(|p| p.id == id) {
                        p.name = name;
                        p.sku = sku;
                        p.notes = notes;
                        p.production_cost = production;
                        p.retail_price = retail;
                        p.shipping_cost = shipping;
                        p.quantity = quantity;
                        p.platform = platform;
                    }
                }
                None => {
                    coll.products.push(Product {
                        id: new_id.unwrap(),
                        name,
                        sku,
                        notes,
                        production_cost: production,
                        retail_price: retail,
                        shipping_cost: shipping,
                        quantity,
                        platform,
                    });
                    added_index = Some(coll.products.len() - 1);
                }
            }
        }
        if let Some(si) = added_index {
            let di = self.storage_to_display(si).unwrap_or(si);
            self.prod_state.select(Some(di));
        }
        self.persist();
        self.focus = Focus::Products;
        self.mode = Mode::Browse;
    }

    // ---- Platform picker -------------------------------------------------

    fn open_pick_platform(&mut self) {
        let current = self.current_product().map(|p| p.platform.clone());
        if let Some(name) = current {
            let idx = self
                .presets
                .iter()
                .position(|p| p.name == name)
                .unwrap_or(0);
            self.pick_state.select(Some(idx));
            self.mode = Mode::PickPlatform;
        } else {
            self.status = "Select a product first".into();
        }
    }

    fn key_pick_platform(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Browse,
            KeyCode::Up | KeyCode::Char('k') => {
                move_sel(&mut self.pick_state, self.presets.len(), -1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                move_sel(&mut self.pick_state, self.presets.len(), 1)
            }
            KeyCode::Enter => self.apply_picked_platform(),
            _ => {}
        }
    }

    fn apply_picked_platform(&mut self) {
        let name = self
            .pick_state
            .selected()
            .and_then(|i| self.presets.get(i))
            .map(|p| p.name.clone());
        let si = self.prod_display_to_storage();
        if let (Some(name), Some(ci), Some(si)) = (name, self.coll_state.selected(), si) {
            if let Some(coll) = self.store.collections.get_mut(ci)
                && let Some(p) = coll.products.get_mut(si)
            {
                p.platform = name;
            }
            self.persist();
            self.status = "Platform updated".into();
        }
        self.mode = Mode::Browse;
    }

    // ---- Compare ---------------------------------------------------------

    fn open_compare(&mut self) {
        if self.current_product().is_some() {
            self.mode = Mode::Compare;
        } else {
            self.status = "Select a product to compare".into();
        }
    }

    fn key_compare(&mut self, key: KeyEvent) {
        if matches!(
            key.code,
            KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('q') | KeyCode::Enter
        ) {
            self.mode = Mode::Browse;
        }
    }

    // ---- Reverse pricing (target margin -> price) ------------------------

    fn open_reverse(&mut self) {
        // Prefill the target with the product's current margin, so the user
        // sees the status quo and nudges from there.
        let data = self
            .current_product()
            .and_then(|p| self.preset_by_name(&p.platform).map(|pr| (p.id, p, pr)))
            .map(|(id, p, pr)| (id, crate::calc::breakdown(p, pr).margin));
        if let Some((id, margin)) = data {
            self.reverse = Some(ReverseForm {
                product_id: id,
                target_margin: TextField::new(&margin.round_dp(1).to_string()),
            });
            self.mode = Mode::Reverse;
        } else {
            self.status = "Select a product to reprice".into();
        }
    }

    fn key_reverse(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.reverse = None;
                self.mode = Mode::Browse;
            }
            KeyCode::Enter => self.apply_reverse(),
            KeyCode::Left => self.with_reverse_field(|t| t.left()),
            KeyCode::Right => self.with_reverse_field(|t| t.right()),
            KeyCode::Home => self.with_reverse_field(|t| t.home()),
            KeyCode::End => self.with_reverse_field(|t| t.end()),
            KeyCode::Backspace => self.with_reverse_field(|t| t.backspace()),
            KeyCode::Delete => self.with_reverse_field(|t| t.delete()),
            KeyCode::Char(c) => self.with_reverse_field(|t| t.insert(c)),
            _ => {}
        }
    }

    fn with_reverse_field(&mut self, f: impl FnOnce(&mut TextField)) {
        if let Some(r) = self.reverse.as_mut() {
            f(&mut r.target_margin);
        }
    }

    /// Compute the suggested price for the current reverse form, if solvable.
    pub fn reverse_suggestion(&self) -> Option<Decimal> {
        let r = self.reverse.as_ref()?;
        let product = self
            .store
            .collections
            .iter()
            .flat_map(|c| &c.products)
            .find(|p| p.id == r.product_id)?;
        let preset = self.preset_by_name(&product.platform)?;
        let target = parse_money(r.target_margin.value());
        crate::calc::suggest_price(
            product.production_cost,
            product.shipping_cost,
            preset,
            target,
        )
    }

    fn apply_reverse(&mut self) {
        let suggestion = self.reverse_suggestion();
        let id = self.reverse.as_ref().map(|r| r.product_id);
        match (suggestion, id) {
            (Some(price), Some(id)) => {
                for coll in &mut self.store.collections {
                    if let Some(p) = coll.products.iter_mut().find(|p| p.id == id) {
                        p.retail_price = price;
                        break;
                    }
                }
                self.persist();
                self.status = format!("Retail set to {}", self.money(price));
                self.reverse = None;
                self.mode = Mode::Browse;
            }
            _ => {
                self.status = "That margin isn't reachable with these fees".into();
            }
        }
    }

    // ---- Spreadsheet import / export --------------------------------------

    fn export_xlsx(&mut self) {
        match crate::xlsx_export::export(&self.store, &self.presets) {
            Ok(path) => {
                self.status = format!("Spreadsheet saved to {}", path.display());
                // Open Explorer with the file selected.
                let _ = std::process::Command::new("explorer")
                    .args(["/select,", &path.to_string_lossy()])
                    .spawn();
            }
            Err(e) => self.status = format!("Spreadsheet export failed: {e}"),
        }
    }

    fn open_import(&mut self) {
        let default = xlsx_import::default_import_path().display().to_string();
        self.import_path = Some(TextField::new(&default));
        self.mode = Mode::ImportPath;
    }

    fn key_import(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.import_path = None;
                self.mode = Mode::Browse;
            }
            KeyCode::Enter => {
                let path = self
                    .import_path
                    .as_ref()
                    .map(|t| PathBuf::from(t.value().trim().trim_matches('"').trim()));
                if let Some(path) = path {
                    self.open_import_map(&path);
                }
            }
            KeyCode::Left => self.with_import_field(|t| t.left()),
            KeyCode::Right => self.with_import_field(|t| t.right()),
            KeyCode::Home => self.with_import_field(|t| t.home()),
            KeyCode::End => self.with_import_field(|t| t.end()),
            KeyCode::Backspace => self.with_import_field(|t| t.backspace()),
            KeyCode::Delete => self.with_import_field(|t| t.delete()),
            KeyCode::Char(c) => self.with_import_field(|t| t.insert(c)),
            _ => {}
        }
    }

    fn with_import_field(&mut self, f: impl FnOnce(&mut TextField)) {
        if let Some(t) = self.import_path.as_mut() {
            f(t);
        }
    }

    /// Parse the file and open the column-mapping screen (or show an error).
    fn open_import_map(&mut self, path: &std::path::Path) {
        match xlsx_import::parse(path) {
            Ok(sheets) => {
                let mapping = xlsx_import::guess_mapping(&sheets[0]);
                let coll_names = sheets.iter().map(|s| TextField::new(&s.name)).collect();
                self.import_map = Some(ImportMapState {
                    sheets,
                    mapping,
                    cursor: 0,
                    platform_idx: 0,
                    coll_names,
                });
                self.import_path = None;
                self.mode = Mode::ImportMap;
            }
            Err(e) => {
                self.error_popup = Some(format!("Couldn't read the spreadsheet\n\n{e}"));
                self.import_path = None;
                self.mode = Mode::Browse;
            }
        }
    }

    /// Number of selectable rows: field rows + platform picker + one name per sheet.
    fn import_map_rows(&self) -> usize {
        let fields = xlsx_import::ImportField::ALL.len();
        match &self.import_map {
            Some(s) => fields + 1 + s.sheets.len(),
            None => fields,
        }
    }

    /// Row index of the platform picker.
    fn import_map_platform_row() -> usize {
        xlsx_import::ImportField::ALL.len()
    }

    fn key_import_map(&mut self, key: KeyEvent) {
        let rows = self.import_map_rows();
        match key.code {
            KeyCode::Esc => {
                self.import_map = None;
                self.mode = Mode::Browse;
            }
            KeyCode::Up | KeyCode::BackTab => {
                if let Some(s) = self.import_map.as_mut() {
                    s.cursor = if s.cursor == 0 { rows - 1 } else { s.cursor - 1 };
                }
            }
            KeyCode::Down | KeyCode::Tab => {
                if let Some(s) = self.import_map.as_mut() {
                    s.cursor = (s.cursor + 1) % rows;
                }
            }
            KeyCode::Left => self.import_map_adjust(false),
            KeyCode::Right => self.import_map_adjust(true),
            KeyCode::Enter => self.apply_import_map(),
            KeyCode::Backspace => self.import_map_text(|t| t.backspace()),
            KeyCode::Delete => self.import_map_text(|t| t.delete()),
            KeyCode::Home => self.import_map_text(|t| t.home()),
            KeyCode::End => self.import_map_text(|t| t.end()),
            KeyCode::Char(c) => self.import_map_text(|t| t.insert(c)),
            _ => {}
        }
    }

    /// ←/→ behaviour depends on which row is selected: field row cycles its
    /// column, platform row cycles the preset, name row moves the text cursor.
    fn import_map_adjust(&mut self, forward: bool) {
        let plat_row = Self::import_map_platform_row();
        let preset_count = self.presets.len();
        let Some(s) = self.import_map.as_mut() else {
            return;
        };
        let fields = xlsx_import::ImportField::ALL;
        if s.cursor < fields.len() {
            let cols = s.sheets[0].num_cols();
            s.mapping.cycle(fields[s.cursor], cols, forward);
        } else if s.cursor == plat_row {
            if preset_count > 0 {
                s.platform_idx = if forward {
                    (s.platform_idx + 1) % preset_count
                } else {
                    (s.platform_idx + preset_count - 1) % preset_count
                };
            }
        } else if let Some(tf) = s.coll_names.get_mut(s.cursor - plat_row - 1) {
            if forward {
                tf.right();
            } else {
                tf.left();
            }
        }
    }

    /// Text editing only applies when a collection-name row is selected.
    fn import_map_text(&mut self, f: impl FnOnce(&mut TextField)) {
        let plat_row = Self::import_map_platform_row();
        let Some(s) = self.import_map.as_mut() else {
            return;
        };
        if s.cursor > plat_row {
            if let Some(tf) = s.coll_names.get_mut(s.cursor - plat_row - 1) {
                f(tf);
            }
        }
    }

    /// Turn the current mapping into per-sheet `(collection name, products)`,
    /// plus a count of rows skipped for having no valid (positive) retail price.
    /// Shared by the live preview and the actual import so they always agree.
    fn build_import_products(
        &self,
        state: &ImportMapState,
    ) -> (Vec<(String, Vec<Product>)>, usize) {
        use xlsx_import::ImportField as IF;
        let platform = self
            .presets
            .get(state.platform_idx)
            .map(|p| p.name.clone())
            .unwrap_or_default();

        let mut out: Vec<(String, Vec<Product>)> = Vec::new();
        let mut skipped = 0usize;

        for (si, sheet) in state.sheets.iter().enumerate() {
            // For sheets after the first, match the chosen column by header text
            // (falling back to the same index) so multi-sheet files line up.
            let resolve = |field: IF| -> Option<usize> {
                let base = state.mapping.get(field)?;
                if si == 0 {
                    return Some(base);
                }
                let header = state.sheets[0].headers.get(base)?;
                sheet.col_for_header(header).or(Some(base))
            };

            let c_name = resolve(IF::Name);
            let c_retail = resolve(IF::Retail);
            let c_sku = resolve(IF::Sku);
            let c_qty = resolve(IF::Quantity);
            let c_cost = resolve(IF::Cost);
            let c_ship = resolve(IF::Shipping);
            let c_notes = resolve(IF::Notes);

            let mut products: Vec<Product> = Vec::new();
            for r in 0..sheet.num_rows() {
                let name = sheet.cell(r, c_name).trim().to_string();
                if name.is_empty() || name.eq_ignore_ascii_case("totals") {
                    continue;
                }
                let retail = parse_money(&sheet.cell(r, c_retail));
                if retail <= Decimal::ZERO {
                    skipped += 1; // has a name but no usable price
                    continue;
                }
                products.push(Product {
                    id: 0, // real id assigned at insert time
                    name,
                    sku: sheet.cell(r, c_sku).trim().to_string(),
                    notes: sheet.cell(r, c_notes).trim().to_string(),
                    production_cost: parse_money(&sheet.cell(r, c_cost)),
                    retail_price: retail,
                    shipping_cost: parse_money(&sheet.cell(r, c_ship)),
                    quantity: sheet.cell(r, c_qty).trim().parse::<u32>().unwrap_or(1).max(1),
                    platform: platform.clone(),
                });
            }

            let base_name = state
                .coll_names
                .get(si)
                .map(|t| t.value().trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| sheet.name.clone());
            out.push((base_name, products));
        }

        (out, skipped)
    }

    /// Live preview/summary for the map screen: counts + the first computed product.
    pub fn import_preview(&self) -> Option<ImportPreview> {
        let state = self.import_map.as_ref()?;
        let (per_sheet, skipped) = self.build_import_products(state);
        let valid = per_sheet.iter().map(|(_, p)| p.len()).sum();
        let first = per_sheet
            .into_iter()
            .flat_map(|(_, p)| p)
            .next()
            .and_then(|p| self.preset_by_name(&p.platform).cloned().map(|pr| (p, pr)));
        Some(ImportPreview {
            valid,
            skipped,
            first,
        })
    }

    fn apply_import_map(&mut self) {
        use xlsx_import::ImportField as IF;
        let state = match self.import_map.take() {
            Some(s) => s,
            None => {
                self.mode = Mode::Browse;
                return;
            }
        };

        // Name and Retail are required for a meaningful import.
        if state.mapping.get(IF::Name).is_none() || state.mapping.get(IF::Retail).is_none() {
            self.error_popup = Some(
                "Map the Name and Retail columns before importing.\n\nUse ←/→ to assign a column to each."
                    .into(),
            );
            self.import_map = Some(state); // keep the screen open so the user can fix it
            return;
        }

        let (per_sheet, skipped) = self.build_import_products(&state);

        let mut added = 0usize;
        let mut colls = 0usize;
        for (base_name, products) in per_sheet {
            if products.is_empty() {
                continue;
            }
            let name = self.unique_collection_name(&base_name);
            let cid = self.store.new_id();
            let mut coll = Collection {
                id: cid,
                name,
                products: Vec::new(),
            };
            for mut p in products {
                p.id = self.store.new_id();
                coll.products.push(p);
                added += 1;
            }
            self.store.collections.push(coll);
            colls += 1;
        }

        if self.coll_state.selected().is_none() && !self.store.collections.is_empty() {
            self.coll_state.select(Some(0));
        }
        self.clamp_selections();
        self.persist();
        self.mode = Mode::Browse;

        if added == 0 {
            let extra = if skipped > 0 {
                format!(" {skipped} row(s) had no valid retail price.")
            } else {
                String::new()
            };
            self.error_popup = Some(format!("No products were imported.{extra}"));
        } else {
            let mut msg = format!("Imported {added} product(s) into {colls} collection(s)");
            if skipped > 0 {
                msg.push_str(&format!(" · skipped {skipped}"));
            }
            self.status = msg;
        }
    }

    /// A collection name not already in use (appends " (2)", " (3)", ...).
    fn unique_collection_name(&self, base: &str) -> String {
        let base = if base.trim().is_empty() {
            "Imported"
        } else {
            base.trim()
        };
        let exists = |n: &str| self.store.collections.iter().any(|c| c.name == n);
        if !exists(base) {
            return base.to_string();
        }
        let mut i = 2;
        loop {
            let candidate = format!("{base} ({i})");
            if !exists(&candidate) {
                return candidate;
            }
            i += 1;
        }
    }

    // ---- Confirm delete --------------------------------------------------

    fn key_confirm(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                if let Some(action) = self.confirm.take() {
                    match action {
                        ConfirmAction::DeleteCollection(id) => {
                            self.store.collections.retain(|c| c.id != id);
                            self.clamp_selections();
                        }
                        ConfirmAction::DeleteProduct(id) => {
                            if let Some(ci) = self.coll_state.selected()
                                && let Some(coll) = self.store.collections.get_mut(ci)
                            {
                                coll.products.retain(|p| p.id != id);
                            }
                            self.clamp_selections();
                        }
                        ConfirmAction::DeletePreset(name) => {
                            self.store.custom_presets.retain(|p| p.name != name);
                            self.rebuild_presets();
                            self.clamp_preset_editor();
                        }
                    }
                    self.persist();
                    self.status = "Deleted".into();
                }
                self.mode = self.before_confirm.clone();
                self.before_confirm = Mode::Browse;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.confirm = None;
                self.mode = self.before_confirm.clone();
                self.before_confirm = Mode::Browse;
            }
            _ => {}
        }
    }

    // ---- Duplicate product -----------------------------------------------

    fn duplicate_product(&mut self) {
        if self.focus != Focus::Products {
            return;
        }
        let (prod_clone, ci) = {
            let ci = match self.coll_state.selected() {
                Some(i) => i,
                None => return,
            };
            let si = match self.prod_display_to_storage() {
                Some(s) => s,
                None => return,
            };
            let coll = match self.store.collections.get(ci) {
                Some(c) => c,
                None => return,
            };
            let p = match coll.products.get(si) {
                Some(p) => p,
                None => return,
            };
            (p.clone(), ci)
        };
        let new_id = self.store.new_id();
        let mut copy = prod_clone;
        copy.id = new_id;
        copy.name = format!("{} (copy)", copy.name);
        let new_si = {
            let coll = &mut self.store.collections[ci];
            coll.products.push(copy);
            coll.products.len() - 1
        };
        let di = self.storage_to_display(new_si).unwrap_or(new_si);
        self.prod_state.select(Some(di));
        self.persist();
        self.status = "Duplicated".into();
    }

    // ---- Move product between collections --------------------------------

    fn open_move_product(&mut self) {
        if self.focus != Focus::Products {
            return;
        }
        let id = match self.current_product().map(|p| p.id) {
            Some(id) => id,
            None => {
                self.status = "Select a product to move".into();
                return;
            }
        };
        let cur_ci = self.coll_state.selected().unwrap_or(0);
        // Count collections other than current; need at least one destination.
        let dest_count = self.store.collections.len().saturating_sub(1);
        if dest_count == 0 {
            self.status = "Create another collection first".into();
            return;
        }
        // Select first destination (skip current collection).
        let first_dest = if cur_ci == 0 { 1 } else { 0 };
        // move_list_state indexes into all collections; map to exclude current later.
        self.move_list_state.select(Some(first_dest.min(self.store.collections.len() - 1)));
        self.move_product_id = Some(id);
        self.mode = Mode::MoveProduct;
    }

    fn key_move_product(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.move_product_id = None;
                self.mode = Mode::Browse;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                move_sel(&mut self.move_list_state, self.store.collections.len(), -1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                move_sel(&mut self.move_list_state, self.store.collections.len(), 1)
            }
            KeyCode::Enter => self.apply_move_product(),
            _ => {}
        }
    }

    fn apply_move_product(&mut self) {
        let product_id = match self.move_product_id.take() {
            Some(id) => id,
            None => { self.mode = Mode::Browse; return; }
        };
        let from_ci = match self.coll_state.selected() {
            Some(i) => i,
            None => { self.mode = Mode::Browse; return; }
        };
        let dest_ci = match self.move_list_state.selected() {
            Some(i) => i,
            None => { self.mode = Mode::Browse; return; }
        };
        if dest_ci == from_ci {
            self.status = "Already in this collection".into();
            self.mode = Mode::Browse;
            return;
        }
        // Remove from source.
        let product = {
            let coll = &mut self.store.collections[from_ci];
            if let Some(pos) = coll.products.iter().position(|p| p.id == product_id) {
                coll.products.remove(pos)
            } else {
                self.mode = Mode::Browse;
                return;
            }
        };
        // Add to destination.
        self.store.collections[dest_ci].products.push(product);
        self.clamp_selections();
        self.persist();
        self.status = format!("Moved to \"{}\"", self.store.collections[dest_ci].name);
        self.mode = Mode::Browse;
    }

    // ---- Sort ------------------------------------------------------------

    fn cycle_sort(&mut self) {
        self.sort_mode = match self.sort_mode {
            SortMode::Default => SortMode::MarginDesc,
            SortMode::MarginDesc => SortMode::MarginAsc,
            SortMode::MarginAsc => SortMode::Name,
            SortMode::Name => SortMode::Default,
        };
        self.status = match self.sort_mode {
            SortMode::Default => "Sort: entry order".into(),
            SortMode::MarginDesc => "Sort: best margin first".into(),
            SortMode::MarginAsc => "Sort: worst margin first".into(),
            SortMode::Name => "Sort: name A→Z".into(),
        };
        // Keep the same product selected across the sort.
        let cur_id = self.current_product().map(|p| p.id);
        if let Some(id) = cur_id {
            // Recompute and find the new display index.
            let di = self
                .sorted_indices()
                .iter()
                .position(|&si| {
                    self.current_collection()
                        .and_then(|c| c.products.get(si))
                        .map(|p| p.id == id)
                        .unwrap_or(false)
                });
            if let Some(di) = di {
                self.prod_state.select(Some(di));
            }
        }
    }

    // ---- Search ----------------------------------------------------------

    fn open_search(&mut self) {
        self.search_query = Some(TextField::new(""));
        self.search_results.clear();
        self.search_result_state.select(None);
        self.mode = Mode::Search;
    }

    fn key_search(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.search_query = None;
                self.search_results.clear();
                self.mode = Mode::Browse;
            }
            KeyCode::Enter => self.apply_search_selection(),
            KeyCode::Up | KeyCode::Char('k') => {
                move_sel(&mut self.search_result_state, self.search_results.len(), -1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                move_sel(&mut self.search_result_state, self.search_results.len(), 1)
            }
            KeyCode::Left => self.with_search_field(|t| t.left()),
            KeyCode::Right => self.with_search_field(|t| t.right()),
            KeyCode::Home => self.with_search_field(|t| t.home()),
            KeyCode::End => self.with_search_field(|t| t.end()),
            KeyCode::Backspace => self.with_search_field(|t| t.backspace()),
            KeyCode::Delete => self.with_search_field(|t| t.delete()),
            KeyCode::Char(c) => self.with_search_field(|t| t.insert(c)),
            _ => {}
        }
    }

    fn with_search_field(&mut self, f: impl FnOnce(&mut TextField)) {
        if let Some(t) = self.search_query.as_mut() {
            f(t);
        }
        self.update_search_results();
    }

    fn update_search_results(&mut self) {
        let query = match &self.search_query {
            Some(t) => t.value().trim().to_lowercase(),
            None => { self.search_results.clear(); return; }
        };
        if query.is_empty() {
            self.search_results.clear();
            self.search_result_state.select(None);
            return;
        }
        self.search_results = self.store.collections
            .iter()
            .enumerate()
            .flat_map(|(ci, coll)| {
                coll.products
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| {
                        p.name.to_lowercase().contains(&query)
                            || p.sku.to_lowercase().contains(&query)
                            || p.notes.to_lowercase().contains(&query)
                    })
                    .map(move |(pi, _)| (ci, pi))
                    .collect::<Vec<_>>()
            })
            .collect();
        if self.search_results.is_empty() {
            self.search_result_state.select(None);
        } else {
            self.search_result_state.select(Some(0));
        }
    }

    fn apply_search_selection(&mut self) {
        let &(ci, si) = match self
            .search_result_state
            .selected()
            .and_then(|i| self.search_results.get(i))
        {
            Some(r) => r,
            None => {
                self.search_query = None;
                self.search_results.clear();
                self.mode = Mode::Browse;
                return;
            }
        };
        self.coll_state.select(Some(ci));
        // Map storage index to display index under current sort.
        let di = self.storage_to_display(si).unwrap_or(si);
        self.prod_state.select(Some(di));
        self.focus = Focus::Products;
        self.search_query = None;
        self.search_results.clear();
        self.mode = Mode::Browse;
    }

    // ---- Settings --------------------------------------------------------

    fn open_settings(&mut self) {
        self.settings_field = Some(TextField::new(&self.store.settings.currency));
        self.mode = Mode::Settings;
    }

    fn key_settings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.settings_field = None;
                self.mode = Mode::Browse;
            }
            KeyCode::Enter => {
                let sym = self
                    .settings_field
                    .take()
                    .map(|t| t.value().trim().to_string())
                    .unwrap_or_else(|| "$".to_string());
                self.store.settings.currency = if sym.is_empty() {
                    "$".to_string()
                } else {
                    sym
                };
                self.persist();
                self.status = "Settings saved".into();
                self.mode = Mode::Browse;
            }
            KeyCode::Left => self.with_settings_field(|t| t.left()),
            KeyCode::Right => self.with_settings_field(|t| t.right()),
            KeyCode::Home => self.with_settings_field(|t| t.home()),
            KeyCode::End => self.with_settings_field(|t| t.end()),
            KeyCode::Backspace => self.with_settings_field(|t| t.backspace()),
            KeyCode::Delete => self.with_settings_field(|t| t.delete()),
            KeyCode::Char(c) => self.with_settings_field(|t| t.insert(c)),
            _ => {}
        }
    }

    fn with_settings_field(&mut self, f: impl FnOnce(&mut TextField)) {
        if let Some(t) = self.settings_field.as_mut() {
            f(t);
        }
    }

    // ---- Sale scenario ---------------------------------------------------

    fn open_sale_input(&mut self) {
        if self.active_sale_pct.is_some() {
            self.active_sale_pct = None;
            self.status = "Sale mode off".into();
            return;
        }
        self.sale_field = Some(TextField::new(""));
        self.fake_target_field = Some(TextField::new(""));
        self.sale_input_focus = false;
        self.mode = Mode::SaleInput;
    }

    fn key_sale_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.sale_field = None;
                self.fake_target_field = None;
                self.mode = Mode::Browse;
            }
            KeyCode::Tab => {
                self.sale_input_focus = !self.sale_input_focus;
            }
            KeyCode::Enter => {
                let pct = self
                    .sale_field
                    .take()
                    .map(|t| parse_money(t.value()))
                    .unwrap_or(Decimal::ZERO);
                let fake_target = self
                    .fake_target_field
                    .take()
                    .map(|t| parse_money(t.value()))
                    .unwrap_or(Decimal::ZERO);

                let msg: String;

                if pct <= Decimal::ZERO || pct >= dec!(100) {
                    msg = "Discount must be 1–99%.".into();
                } else if !fake_target.is_zero() {
                    // Fake sale: just a calculator — don't change the right-side breakdown.
                    // The user's actual selling price stays the same; we tell them what
                    // higher list price to set so a "X% off" badge lands on their real price.
                    let list_price =
                        (fake_target / (dec!(100) - pct) * dec!(100)).round_dp(2);
                    msg = format!(
                        "Fake sale: list at {} → show -{:.0}% off → customer pays {}",
                        self.money(list_price),
                        pct,
                        self.money(fake_target),
                    );
                } else {
                    // Real sale mode: applies the discount to the breakdown on the right.
                    self.active_sale_pct = Some(pct.round_dp(1));
                    msg = format!("Sale mode on: -{:.0}% — press % again to clear", pct);
                }

                self.status = msg;
                self.mode = Mode::Browse;
            }
            KeyCode::Left => self.sale_active_field(|t| t.left()),
            KeyCode::Right => self.sale_active_field(|t| t.right()),
            KeyCode::Home => self.sale_active_field(|t| t.home()),
            KeyCode::End => self.sale_active_field(|t| t.end()),
            KeyCode::Backspace => self.sale_active_field(|t| t.backspace()),
            KeyCode::Delete => self.sale_active_field(|t| t.delete()),
            KeyCode::Char(c) => self.sale_active_field(|t| t.insert(c)),
            _ => {}
        }
    }

    fn sale_active_field(&mut self, f: impl FnOnce(&mut TextField)) {
        if self.sale_input_focus {
            if let Some(t) = self.fake_target_field.as_mut() {
                f(t);
            }
        } else if let Some(t) = self.sale_field.as_mut() {
            f(t);
        }
    }

    // ---- Wholesale view --------------------------------------------------

    fn open_wholesale_input(&mut self) {
        if self.active_wholesale_pct.is_some() {
            self.active_wholesale_pct = None;
            self.status = "Wholesale mode off".into();
            return;
        }
        self.wholesale_field = Some(TextField::new("50"));
        self.mode = Mode::WholesaleInput;
    }

    fn key_wholesale_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.wholesale_field = None;
                self.mode = Mode::Browse;
            }
            KeyCode::Enter => {
                let pct = self
                    .wholesale_field
                    .take()
                    .map(|t| parse_money(t.value()))
                    .unwrap_or(Decimal::ZERO);
                if pct > Decimal::ZERO && pct < dec!(100) {
                    self.active_wholesale_pct = Some(pct.round_dp(1));
                    self.status = format!(
                        "Wholesale mode on: {:.0}% of retail — press w again to clear",
                        pct
                    );
                } else {
                    self.status = "Wholesale % must be 1–99.".into();
                }
                self.mode = Mode::Browse;
            }
            KeyCode::Left => self.with_wholesale_field(|t| t.left()),
            KeyCode::Right => self.with_wholesale_field(|t| t.right()),
            KeyCode::Home => self.with_wholesale_field(|t| t.home()),
            KeyCode::End => self.with_wholesale_field(|t| t.end()),
            KeyCode::Backspace => self.with_wholesale_field(|t| t.backspace()),
            KeyCode::Delete => self.with_wholesale_field(|t| t.delete()),
            KeyCode::Char(c) => self.with_wholesale_field(|t| t.insert(c)),
            _ => {}
        }
    }

    fn with_wholesale_field(&mut self, f: impl FnOnce(&mut TextField)) {
        if let Some(t) = self.wholesale_field.as_mut() {
            f(t);
        }
    }

    // ---- Custom preset editor -------------------------------------------

    fn key_preset_editor(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('f') => self.mode = Mode::Browse,
            KeyCode::Up | KeyCode::Char('k') => {
                move_sel(
                    &mut self.preset_editor_state,
                    self.store.custom_presets.len(),
                    -1,
                )
            }
            KeyCode::Down | KeyCode::Char('j') => {
                move_sel(
                    &mut self.preset_editor_state,
                    self.store.custom_presets.len(),
                    1,
                )
            }
            KeyCode::Char('n') => self.open_preset_form(None),
            KeyCode::Char('e') | KeyCode::Enter => {
                let name = self
                    .preset_editor_state
                    .selected()
                    .and_then(|i| self.store.custom_presets.get(i))
                    .map(|p| p.name.clone());
                if let Some(name) = name {
                    self.open_preset_form(Some(name));
                }
            }
            KeyCode::Char('d') => {
                let name = self
                    .preset_editor_state
                    .selected()
                    .and_then(|i| self.store.custom_presets.get(i))
                    .map(|p| p.name.clone());
                if let Some(name) = name {
                    self.confirm = Some(ConfirmAction::DeletePreset(name));
                    self.before_confirm = Mode::PresetEditor;
                    self.mode = Mode::Confirm;
                }
            }
            _ => {}
        }
    }

    fn open_preset_form(&mut self, existing_name: Option<String>) {
        let form = if let Some(ref name) = existing_name {
            match self.store.custom_presets.iter().find(|p| &p.name == name) {
                Some(p) => CustomPresetForm {
                    editing: existing_name.clone(),
                    name: TextField::new(&p.name),
                    note: TextField::new(&p.note),
                    lines: if p.lines.is_empty() {
                        vec![PresetLineForm::blank()]
                    } else {
                        p.lines
                            .iter()
                            .map(|l| PresetLineForm {
                                label: TextField::new(&l.label),
                                percent: TextField::new(&l.percent.to_string()),
                                flat: TextField::new(&l.flat.to_string()),
                            })
                            .collect()
                    },
                    field: PresetFormField::Name,
                },
                None => return,
            }
        } else {
            CustomPresetForm {
                editing: None,
                name: TextField::new(""),
                note: TextField::new(""),
                lines: vec![PresetLineForm::blank()],
                field: PresetFormField::Name,
            }
        };
        self.preset_form = Some(form);
        self.mode = Mode::PresetForm;
    }

    fn key_preset_form(&mut self, key: KeyEvent) {
        // Ctrl+N / Ctrl+D: add or remove a fee line (captured before char dispatch).
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('n') => {
                    if let Some(f) = self.preset_form.as_mut() {
                        if f.lines.len() < 6 {
                            f.lines.push(PresetLineForm::blank());
                        } else {
                            self.status = "Maximum 6 fee lines".into();
                        }
                    }
                    return;
                }
                KeyCode::Char('d') => {
                    if let Some(f) = self.preset_form.as_mut()
                        && f.lines.len() > 1
                    {
                        f.lines.pop();
                        let nlines = f.lines.len();
                        // Move focus back if it pointed past the end.
                        let past_end = match &f.field {
                            PresetFormField::LineLabel(i)
                            | PresetFormField::LinePercent(i)
                            | PresetFormField::LineFlat(i) => *i >= nlines,
                            _ => false,
                        };
                        if past_end {
                            f.field = PresetFormField::LineFlat(nlines - 1);
                        }
                    }
                    return;
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Esc => {
                self.preset_form = None;
                self.mode = Mode::PresetEditor;
                self.status = "Cancelled".into();
            }
            KeyCode::Enter => self.commit_preset_form(),
            KeyCode::Tab | KeyCode::Down => {
                if let Some(f) = self.preset_form.as_mut() {
                    let nlines = f.lines.len();
                    let cur = f.field.clone();
                    f.field = next_preset_field(&cur, nlines);
                }
            }
            KeyCode::BackTab | KeyCode::Up => {
                if let Some(f) = self.preset_form.as_mut() {
                    let nlines = f.lines.len();
                    let cur = f.field.clone();
                    f.field = prev_preset_field(&cur, nlines);
                }
            }
            KeyCode::Left => self.with_preset_text(|t| t.left()),
            KeyCode::Right => self.with_preset_text(|t| t.right()),
            KeyCode::Home => self.with_preset_text(|t| t.home()),
            KeyCode::End => self.with_preset_text(|t| t.end()),
            KeyCode::Backspace => self.with_preset_text(|t| t.backspace()),
            KeyCode::Delete => self.with_preset_text(|t| t.delete()),
            KeyCode::Char(c) => self.with_preset_text(|t| t.insert(c)),
            _ => {}
        }
    }

    fn with_preset_text(&mut self, f: impl FnOnce(&mut TextField)) {
        if let Some(form) = self.preset_form.as_mut() {
            let field = form.field.clone();
            match field {
                PresetFormField::Name => f(&mut form.name),
                PresetFormField::Note => f(&mut form.note),
                PresetFormField::LineLabel(i) => {
                    if let Some(l) = form.lines.get_mut(i) {
                        f(&mut l.label);
                    }
                }
                PresetFormField::LinePercent(i) => {
                    if let Some(l) = form.lines.get_mut(i) {
                        f(&mut l.percent);
                    }
                }
                PresetFormField::LineFlat(i) => {
                    if let Some(l) = form.lines.get_mut(i) {
                        f(&mut l.flat);
                    }
                }
            }
        }
    }

    fn commit_preset_form(&mut self) {
        let name = match self.preset_form.as_ref() {
            Some(f) => f.name.value().trim().to_string(),
            None => return,
        };
        if name.is_empty() {
            self.status = "Preset name can't be empty".into();
            return;
        }

        let form = self.preset_form.take().unwrap();
        let editing = form.editing.clone();
        let note = form.note.value().trim().to_string();
        let lines: Vec<FeeLine> = form
            .lines
            .iter()
            .map(|l| FeeLine {
                label: l.label.value().trim().to_string(),
                percent: parse_money(l.percent.value()),
                flat: parse_money(l.flat.value()),
                min_price: None,
                max_price: None,
            })
            .collect();

        let preset = FeePreset {
            name: name.clone(),
            category: Category::Custom,
            lines,
            note,
        };

        match editing {
            Some(ref old_name) => {
                // If name changed, update products pointing at the old name.
                if old_name != &name {
                    for coll in &mut self.store.collections {
                        for prod in &mut coll.products {
                            if prod.platform == *old_name {
                                prod.platform = name.clone();
                            }
                        }
                    }
                }
                if let Some(p) = self
                    .store
                    .custom_presets
                    .iter_mut()
                    .find(|p| &p.name == old_name)
                {
                    *p = preset;
                }
            }
            None => {
                self.store.custom_presets.push(preset);
                let idx = self.store.custom_presets.len() - 1;
                self.preset_editor_state.select(Some(idx));
            }
        }

        self.rebuild_presets();
        self.persist();
        self.mode = Mode::PresetEditor;
        self.status = "Preset saved".into();
    }

    // ---- Status bar hint -------------------------------------------------

    pub fn sort_label(&self) -> &'static str {
        match self.sort_mode {
            SortMode::Default => "",
            SortMode::MarginDesc => " [↓ margin]",
            SortMode::MarginAsc => " [↑ margin]",
            SortMode::Name => " [A-Z]",
        }
    }

    pub fn status_hint(&self) -> &'static str {
        match self.mode {
            Mode::Browse => match self.focus {
                Focus::Collections => {
                    " ↑↓ navigate · n new · e rename · d delete · / search · g settings · f fee presets · X export spreadsheet · i import spreadsheet · ? help"
                }
                Focus::Products => {
                    " ↑↓ navigate · n new · e edit · d delete · u duplicate · m move · s sort · c compare · p platform · r reprice · % sale · w wholesale · / search · ? help"
                }
            },
            Mode::ProductForm => {
                " Tab / ↑↓ next field · ← → move cursor / toggle · Enter save · Esc cancel"
            }
            Mode::CollectionForm => " Type a name · Enter save · Esc cancel",
            Mode::PickPlatform => " ↑↓ navigate · Enter apply · Esc cancel",
            Mode::Compare => " Comparing all platforms by margin · Esc or c to close",
            Mode::Reverse => " Type a target margin % · Enter to set the price · Esc cancel",
            Mode::ImportPath => " Type the path to a .xlsx file · Enter to continue · Esc cancel",
            Mode::ImportMap => {
                " ↑↓ choose field · ←→ pick column · Enter import · Esc cancel"
            }
            Mode::Confirm => " y to confirm · n to cancel",
            Mode::Help => " Esc to close help",
            Mode::PresetEditor => " ↑↓ navigate · n new preset · e edit · d delete · Esc close",
            Mode::PresetForm => {
                " Tab / ↑↓ next field · Ctrl+N add fee line · Ctrl+D remove last line · Enter save · Esc cancel"
            }
            Mode::MoveProduct => " ↑↓ choose destination collection · Enter to move · Esc cancel",
            Mode::Search => " Type to search all products · ↑↓ results · Enter to jump · Esc close",
            Mode::Settings => " Type a currency symbol · Enter save · Esc cancel",
            Mode::SaleInput => " Tab switches fields · Enter apply · Esc cancel",
            Mode::WholesaleInput => " Type the wholesale % of retail (e.g. 50 = half price) · Enter apply · Esc cancel",
        }
    }
}
