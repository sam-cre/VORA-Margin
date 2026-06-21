//! Core data structures for VORA-Margin.
//!
//! Everything here serializes to a single local JSON file (see `storage`).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// What kind of fee source a preset represents. Used for grouping in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Category {
    /// A payment processor you'd use on your own store (Stripe, Square...).
    Processor,
    /// A marketplace that bundles selling + processing fees (Etsy, Depop...).
    Marketplace,
    /// A user-defined preset.
    Custom,
}

impl Category {
    pub fn label(&self) -> &'static str {
        match self {
            Category::Processor => "Processor",
            Category::Marketplace => "Marketplace",
            Category::Custom => "Custom",
        }
    }
}

/// A single component of a fee: a percentage of the sale price plus a flat amount.
///
/// Most real-world fees are a stack of these. Stripe is one line
/// (2.9% + $0.30); Etsy is several (transaction %, processing %, listing flat).
/// `min_price` / `max_price` support tiered fees (e.g. Poshmark: flat $2.95 under
/// $15, 20% at $15+). A line with `max_price = Some(15)` only applies when the
/// retail price is strictly less than $15. `min_price = Some(15)` means ≥ $15.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeLine {
    pub label: String,
    /// Percentage of the sale price, e.g. `2.9` means 2.9%.
    pub percent: Decimal,
    /// Flat per-transaction amount, e.g. `0.30`.
    pub flat: Decimal,
    /// Line only applies when retail price ≥ this value.
    #[serde(default)]
    pub min_price: Option<Decimal>,
    /// Line only applies when retail price < this value.
    #[serde(default)]
    pub max_price: Option<Decimal>,
}

/// A named, reusable fee structure (a "platform").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeePreset {
    pub name: String,
    pub category: Category,
    pub lines: Vec<FeeLine>,
    /// Short human note shown in the UI (e.g. "verified 2026" or "verify").
    pub note: String,
}

/// Default quantity for products loaded from older stores that predate the
/// field (serde fills missing fields with this).
fn default_quantity() -> u32 {
    1
}

/// A single product the user is pricing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub id: u64,
    pub name: String,
    pub sku: String,
    /// Per-unit production cost. For bulk orders this is the batch total
    /// divided by `quantity` (the form's bulk helper does that split).
    pub production_cost: Decimal,
    pub retail_price: Decimal,
    pub shipping_cost: Decimal,
    /// Units in this batch / order (1 for print-on-demand). Drives the
    /// collection totals; per-unit margin is independent of it.
    #[serde(default = "default_quantity")]
    pub quantity: u32,
    /// Name of the `FeePreset` this product sells through.
    pub platform: String,
    /// Optional note (supplier, material, link, etc.).
    #[serde(default)]
    pub notes: String,
}

/// User-level app settings, stored alongside the product data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Currency symbol shown in all money displays (default "$").
    #[serde(default = "default_currency")]
    pub currency: String,
}

fn default_currency() -> String {
    "$".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            currency: default_currency(),
        }
    }
}

/// A named group of products (a "folder" of products).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub products: Vec<Product>,
}

/// The entire on-disk state of the app.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Store {
    /// Monotonic id counter so every product/collection gets a stable id.
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub collections: Vec<Collection>,
    /// User-defined presets, merged with the built-ins at runtime.
    #[serde(default)]
    pub custom_presets: Vec<FeePreset>,
    /// App-level settings (currency symbol, etc.).
    #[serde(default)]
    pub settings: Settings,
}

impl Store {
    /// Hand out a fresh unique id.
    pub fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}
