//! Export the product catalogue to a formatted .xlsx spreadsheet.
//!
//! One worksheet per collection, with bold colour-coded headers, currency
//! formatting, margin cells colour-coded red/yellow/green, and a summary
//! row totalling revenue, cost, fees, profit and blended margin.

use crate::calc;
use crate::model::{FeePreset, Store};
use anyhow::{Context, Result};
use directories::UserDirs;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use rust_xlsxwriter::{Color, Format, Workbook};
use std::path::{Path, PathBuf};

// Palette — matches the TUI colours so the sheet feels like the same product.
const ACCENT_BG: u32 = 0x818CF8;
const WHITE_FG: u32 = 0xFFFFFF;
const GREEN_BG: u32 = 0x34D399;
const YELLOW_BG: u32 = 0xFBBF24;
const RED_BG: u32 = 0xF87171;
const SUMMARY_BG: u32 = 0xE2E8F0;

fn base_dir() -> PathBuf {
    UserDirs::new()
        .and_then(|u| u.download_dir().map(Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn default_export_path() -> PathBuf {
    base_dir().join("vora-margin-export.xlsx")
}

/// `Decimal` → `f64` for writing into Excel cells.
fn f(d: Decimal) -> f64 {
    d.to_f64().unwrap_or(0.0)
}

/// Background colour matching the TUI margin traffic-light.
fn margin_bg(margin: Decimal) -> u32 {
    if margin < dec!(20) {
        RED_BG
    } else if margin < dec!(50) {
        YELLOW_BG
    } else {
        GREEN_BG
    }
}

/// Sanitise a collection name so it's safe as an Excel sheet name
/// (max 31 chars, no [ ] : * ? / \).
fn sheet_name(raw: &str) -> String {
    raw.chars()
        .map(|c| match c {
            '[' | ']' | ':' | '*' | '?' | '/' | '\\' => '-',
            c => c,
        })
        .take(31)
        .collect()
}

pub fn export(store: &Store, presets: &[FeePreset]) -> Result<PathBuf> {
    let path = default_export_path();
    let mut wb = Workbook::new();

    // ---- Shared formats -------------------------------------------------
    let hdr = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(ACCENT_BG))
        .set_font_color(Color::RGB(WHITE_FG));

    // Money: two decimal places, comma separator, no symbol
    // (currency symbol comes from the user's own locale in Excel).
    let money = Format::new().set_num_format("#,##0.00");

    let sum_label = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(SUMMARY_BG));
    let sum_money = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(SUMMARY_BG))
        .set_num_format("#,##0.00");
    let sum_pct = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(SUMMARY_BG))
        .set_num_format("0.0\"%\"");

    // ---- One sheet per collection ---------------------------------------
    for coll in &store.collections {
        let ws = wb.add_worksheet();
        ws.set_name(&sheet_name(&coll.name))
            .with_context(|| format!("invalid sheet name for collection '{}'", coll.name))?;

        // Column widths (characters).
        let col_widths: &[(u16, f64)] = &[
            (0, 26.0), // Name
            (1, 14.0), // SKU
            (2, 22.0), // Platform
            (3, 12.0), // Cost
            (4, 12.0), // Shipping
            (5, 12.0), // Retail
            (6, 12.0), // Fees
            (7, 12.0), // Profit/unit
            (8, 10.0), // Margin %
            (9, 28.0), // Notes
        ];
        for &(col, w) in col_widths {
            ws.set_column_width(col, w)?;
        }

        // Header row.
        let headers = [
            "Name",
            "SKU",
            "Platform",
            "Cost",
            "Shipping",
            "Retail",
            "Fees",
            "Profit / unit",
            "Margin %",
            "Notes",
        ];
        for (c, h) in headers.iter().enumerate() {
            ws.write_with_format(0, c as u16, *h, &hdr)?;
        }

        // ---- Product rows -----------------------------------------------
        let mut t_revenue = Decimal::ZERO;
        let mut t_cost = Decimal::ZERO;
        let mut t_shipping = Decimal::ZERO;
        let mut t_fees = Decimal::ZERO;
        let mut t_profit = Decimal::ZERO;

        for (i, p) in coll.products.iter().enumerate() {
            let row = (i + 1) as u32;
            let preset = presets.iter().find(|pr| pr.name == p.platform);
            let bd = preset.map(|pr| calc::breakdown(p, pr));
            let fees = bd.as_ref().map(|b| b.total_fees).unwrap_or(Decimal::ZERO);
            let profit = bd.as_ref().map(|b| b.profit).unwrap_or(Decimal::ZERO);
            let margin = bd.as_ref().map(|b| b.margin).unwrap_or(Decimal::ZERO);

            let qty = Decimal::from(p.quantity);
            t_revenue += p.retail_price * qty;
            t_cost += p.production_cost * qty;
            t_shipping += p.shipping_cost * qty;
            t_fees += fees * qty;
            t_profit += profit * qty;

            // Per-row colour formats for profit and margin.
            let bg = Color::RGB(margin_bg(margin));
            let profit_fmt = Format::new().set_num_format("#,##0.00").set_background_color(bg);
            let margin_fmt = Format::new().set_num_format("0.0\"%\"").set_background_color(bg);

            ws.write(row, 0, p.name.as_str())?;
            ws.write(row, 1, p.sku.as_str())?;
            ws.write(row, 2, p.platform.as_str())?;
            ws.write_with_format(row, 3, f(p.production_cost), &money)?;
            ws.write_with_format(row, 4, f(p.shipping_cost), &money)?;
            ws.write_with_format(row, 5, f(p.retail_price), &money)?;
            ws.write_with_format(row, 6, f(fees), &money)?;
            ws.write_with_format(row, 7, f(profit), &profit_fmt)?;
            ws.write_with_format(row, 8, f(margin), &margin_fmt)?;
            ws.write(row, 9, p.notes.as_str())?;
        }

        // ---- Summary row ------------------------------------------------
        let sum_row = (coll.products.len() + 1) as u32;
        let blended = if t_revenue.is_zero() {
            Decimal::ZERO
        } else {
            t_profit / t_revenue * dec!(100)
        };

        // Blank cells get the summary background so the row looks solid.
        for c in 0u16..10 {
            ws.write_with_format(sum_row, c, "", &sum_label)?;
        }
        ws.write_with_format(sum_row, 0, "TOTALS", &sum_label)?;
        ws.write_with_format(sum_row, 3, f(t_cost), &sum_money)?;
        ws.write_with_format(sum_row, 4, f(t_shipping), &sum_money)?;
        ws.write_with_format(sum_row, 5, f(t_revenue), &sum_money)?;
        ws.write_with_format(sum_row, 6, f(t_fees), &sum_money)?;
        ws.write_with_format(sum_row, 7, f(t_profit), &sum_money)?;
        ws.write_with_format(sum_row, 8, f(blended), &sum_pct)?;
    }

    // If the store is empty, add a placeholder sheet so the file opens cleanly.
    if store.collections.is_empty() {
        let ws = wb.add_worksheet();
        ws.write(0, 0, "No collections yet — add products in VORA·Margin first.")?;
    }

    wb.save(&path)
        .with_context(|| format!("saving spreadsheet to {}", path.display()))?;

    Ok(path)
}
