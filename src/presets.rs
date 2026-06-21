//! Built-in fee presets.
//!
//! Rates marked "verified 2026" were checked against current published
//! pricing in June 2026. Rates marked "verify" are reasonable defaults that
//! should be re-confirmed — fees change often, especially per-country.
//! Users can also add their own presets, which get merged with these.

use crate::model::{Category, FeeLine, FeePreset};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

fn line(label: &str, percent: Decimal, flat: Decimal) -> FeeLine {
    FeeLine {
        label: label.to_string(),
        percent,
        flat,
        min_price: None,
        max_price: None,
    }
}

/// Fee line that only applies when retail price ≥ `min`.
fn line_above(label: &str, percent: Decimal, flat: Decimal, min: Decimal) -> FeeLine {
    FeeLine {
        label: label.to_string(),
        percent,
        flat,
        min_price: Some(min),
        max_price: None,
    }
}

/// Fee line that only applies when retail price < `max`.
fn line_below(label: &str, percent: Decimal, flat: Decimal, max: Decimal) -> FeeLine {
    FeeLine {
        label: label.to_string(),
        percent,
        flat,
        min_price: None,
        max_price: Some(max),
    }
}

fn preset(name: &str, category: Category, note: &str, lines: Vec<FeeLine>) -> FeePreset {
    FeePreset {
        name: name.to_string(),
        category,
        lines,
        note: note.to_string(),
    }
}

/// The full set of built-in presets, ordered roughly by how brands use them.
pub fn builtin_presets() -> Vec<FeePreset> {
    use Category::*;
    vec![
        // ---- Payment processors (your own store / in person) ----
        preset(
            "Stripe (Online)",
            Processor,
            "2.9% + $0.30 · verified · last checked June 2026",
            vec![line("Processing", dec!(2.9), dec!(0.30))],
        ),
        preset(
            "Stripe (In-person)",
            Processor,
            "2.7% + $0.05 · verified · last checked June 2026",
            vec![line("Processing", dec!(2.7), dec!(0.05))],
        ),
        preset(
            "Square (Online)",
            Processor,
            "3.3% + $0.30 · verified (rate rose Jan 2026) · last checked June 2026",
            vec![line("Processing", dec!(3.3), dec!(0.30))],
        ),
        preset(
            "Square (In-person)",
            Processor,
            "2.6% + $0.15 · verified · last checked June 2026",
            vec![line("Processing", dec!(2.6), dec!(0.15))],
        ),
        preset(
            "PayPal (Goods & Services)",
            Processor,
            "3.49% + $0.49 · verified · ⚠ rate varies by checkout method · last checked June 2026",
            vec![line("Processing", dec!(3.49), dec!(0.49))],
        ),
        preset(
            "Shopify Payments (Basic)",
            Processor,
            "2.9% + $0.30 · ⚠ Basic plan ($39/mo) only — other plans differ · last checked June 2026",
            vec![line("Processing", dec!(2.9), dec!(0.30))],
        ),
        preset(
            "Direct / Cash",
            Processor,
            "No processing fee · in-person cash or direct bank transfer",
            vec![line("None", dec!(0), dec!(0))],
        ),
        // ---- Marketplaces (selling + processing bundled) ----
        preset(
            "Depop",
            Marketplace,
            "0% seller fee + 3.3% + $0.45 · verified · last checked June 2026",
            vec![line("Processing", dec!(3.3), dec!(0.45))],
        ),
        preset(
            "Vinted",
            Marketplace,
            "0% seller fee (buyer pays protection) · ⚠ US only — rates vary by country · last checked June 2026",
            vec![line("Seller fee", dec!(0), dec!(0))],
        ),
        preset(
            "Poshmark",
            Marketplace,
            "Flat $2.95 under $15, 20% at $15+ · verified · last checked June 2026",
            vec![
                line_below("Commission (<$15)", dec!(0), dec!(2.95), dec!(15)),
                line_above("Commission (≥$15)", dec!(20), dec!(0), dec!(15)),
            ],
        ),
        preset(
            "Etsy",
            Marketplace,
            "6.5% + 3% + $0.25 processing + $0.20 listing · verified · last checked June 2026",
            vec![
                line("Transaction", dec!(6.5), dec!(0)),
                line("Processing", dec!(3.0), dec!(0.25)),
                line("Listing", dec!(0), dec!(0.20)),
            ],
        ),
        preset(
            "eBay (Clothing)",
            Marketplace,
            "13.6% + $0.40 · verified · ⚠ subcategory rates vary (e.g. handbags, shoes) · last checked June 2026",
            vec![line("Final value", dec!(13.6), dec!(0.40))],
        ),
        preset(
            "Grailed",
            Marketplace,
            "9% + 3.49% + $0.49 processing · verified · last checked June 2026",
            vec![
                line("Commission", dec!(9), dec!(0)),
                line("Processing", dec!(3.49), dec!(0.49)),
            ],
        ),
        preset(
            "Mercari",
            Marketplace,
            "10% only · verified · processing moved to buyer · ⚠ fee structure volatile · last checked June 2026",
            vec![line("Selling fee", dec!(10), dec!(0))],
        ),
    ]
}
