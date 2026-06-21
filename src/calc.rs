//! The pricing engine: turns a product + a fee preset into a margin breakdown.
//!
//! All math uses `Decimal` to avoid floating-point rounding errors on money.

use crate::model::{Collection, FeePreset, Product};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// A fully itemized result for one product on one platform.
pub struct Breakdown {
    pub production: Decimal,
    pub shipping: Decimal,
    /// (label, amount) for each fee component, so the UI can itemize them.
    pub fee_lines: Vec<(String, Decimal)>,
    pub total_fees: Decimal,
    pub retail: Decimal,
    pub profit: Decimal,
    /// Net margin as a percentage of the retail price.
    pub margin: Decimal,
}

/// Compute the per-line and total fee for a given sale price on a preset.
/// Lines with `min_price`/`max_price` thresholds are skipped when the retail
/// price falls outside their active range.
pub fn fees_for(retail: Decimal, preset: &FeePreset) -> (Vec<(String, Decimal)>, Decimal) {
    let mut lines = Vec::with_capacity(preset.lines.len());
    let mut total = Decimal::ZERO;
    for fl in &preset.lines {
        if fl.max_price.map(|m| retail >= m).unwrap_or(false) {
            continue; // retail ≥ max_price → this line doesn't apply
        }
        if fl.min_price.map(|m| retail < m).unwrap_or(false) {
            continue; // retail < min_price → this line doesn't apply
        }
        let amount = retail * fl.percent / dec!(100) + fl.flat;
        total += amount;
        lines.push((fl.label.clone(), amount));
    }
    (lines, total)
}

/// Full breakdown for a product sold through a preset.
pub fn breakdown(product: &Product, preset: &FeePreset) -> Breakdown {
    let retail = product.retail_price;
    let (fee_lines, total_fees) = fees_for(retail, preset);
    let profit = retail - product.production_cost - product.shipping_cost - total_fees;
    let margin = if retail.is_zero() {
        Decimal::ZERO
    } else {
        profit / retail * dec!(100)
    };
    Breakdown {
        production: product.production_cost,
        shipping: product.shipping_cost,
        fee_lines,
        total_fees,
        retail,
        profit,
        margin,
    }
}

/// Reverse mode: given a target net margin (as a percent), suggest a retail
/// price that achieves it after all fees.
///
/// Handles tiered fee structures (e.g. Poshmark's flat $2.95 under $15 vs 20%
/// at $15+) by solving each price tier separately and returning the solution
/// that is consistent with its own tier's boundaries.
///
/// Returns `None` if no tier yields a solvable, consistent price.
pub fn suggest_price(
    production: Decimal,
    shipping: Decimal,
    preset: &FeePreset,
    target_margin_pct: Decimal,
) -> Option<Decimal> {
    // Collect distinct threshold values from all fee lines.
    let mut thresholds: Vec<Decimal> = preset
        .lines
        .iter()
        .flat_map(|l| [l.min_price, l.max_price])
        .flatten()
        .collect();
    thresholds.sort();
    thresholds.dedup();

    // Build half-open intervals [lo, hi) covering all possible prices.
    let mut intervals: Vec<(Option<Decimal>, Option<Decimal>)> = Vec::new();
    if thresholds.is_empty() {
        intervals.push((None, None));
    } else {
        intervals.push((None, Some(thresholds[0])));
        for w in thresholds.windows(2) {
            intervals.push((Some(w[0]), Some(w[1])));
        }
        intervals.push((Some(*thresholds.last().unwrap()), None));
    }

    let m = target_margin_pct / dec!(100);

    for (lo, hi) in intervals {
        let sum_pct: Decimal = preset
            .lines
            .iter()
            .filter(|l| fee_line_applies_in_interval(l, lo, hi))
            .map(|l| l.percent)
            .sum();
        let sum_flat: Decimal = preset
            .lines
            .iter()
            .filter(|l| fee_line_applies_in_interval(l, lo, hi))
            .map(|l| l.flat)
            .sum();

        let p = sum_pct / dec!(100);
        let denom = dec!(1) - p - m;
        if denom <= Decimal::ZERO {
            continue; // not solvable in this tier
        }
        let price = ((production + shipping + sum_flat) / denom).round_dp(2);

        // Accept only if the price actually falls within [lo, hi).
        let above_lo = lo.map(|l| price >= l).unwrap_or(true);
        let below_hi = hi.map(|h| price < h).unwrap_or(true);
        if above_lo && below_hi {
            return Some(price);
        }
    }

    None
}

/// Returns `true` if a fee line applies for every price in the half-open
/// interval `[lo, hi)`. Used by `suggest_price` to determine which lines
/// contribute to the linear equation for each price tier.
fn fee_line_applies_in_interval(
    line: &crate::model::FeeLine,
    lo: Option<Decimal>,
    hi: Option<Decimal>,
) -> bool {
    // Line applies when: price >= min_price (if set) AND price < max_price (if set).
    // For the whole interval [lo, hi):
    //   • lo must be >= min_price so the interval starts in the active range.
    //   • hi must be <= max_price so the interval ends before the line switches off.
    let above_min = match line.min_price {
        None => true,
        Some(min) => lo.map(|l| l >= min).unwrap_or(false),
    };
    let below_max = match line.max_price {
        None => true,
        Some(max) => hi.map(|h| h <= max).unwrap_or(false),
    };
    above_min && below_max
}

/// Aggregate, quantity-weighted economics for a whole collection — what you'd
/// see if you sold every unit of every product in it ("sold-out" figures).
pub struct CollectionTotals {
    pub products: usize,
    pub units: u64,
    pub revenue: Decimal,
    pub cost_of_goods: Decimal,
    pub shipping: Decimal,
    pub fees: Decimal,
    pub profit: Decimal,
    /// Blended margin across the collection (profit / revenue).
    pub margin: Decimal,
}

pub fn collection_totals(collection: &Collection, presets: &[FeePreset]) -> CollectionTotals {
    let mut t = CollectionTotals {
        products: collection.products.len(),
        units: 0,
        revenue: Decimal::ZERO,
        cost_of_goods: Decimal::ZERO,
        shipping: Decimal::ZERO,
        fees: Decimal::ZERO,
        profit: Decimal::ZERO,
        margin: Decimal::ZERO,
    };
    for p in &collection.products {
        let qty = Decimal::from(p.quantity);
        t.units += p.quantity as u64;
        let unit_fees = match presets.iter().find(|x| x.name == p.platform) {
            Some(preset) => fees_for(p.retail_price, preset).1,
            None => Decimal::ZERO,
        };
        t.revenue += p.retail_price * qty;
        t.cost_of_goods += p.production_cost * qty;
        t.shipping += p.shipping_cost * qty;
        t.fees += unit_fees * qty;
    }
    t.profit = t.revenue - t.cost_of_goods - t.shipping - t.fees;
    t.margin = if t.revenue.is_zero() {
        Decimal::ZERO
    } else {
        t.profit / t.revenue * dec!(100)
    };
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Category, FeeLine, FeePreset, Product};

    fn stripe() -> FeePreset {
        FeePreset {
            name: "Stripe".into(),
            category: Category::Processor,
            lines: vec![FeeLine {
                label: "Processing".into(),
                percent: dec!(2.9),
                flat: dec!(0.30),
                min_price: None,
                max_price: None,
            }],
            note: String::new(),
        }
    }

    fn product(production: Decimal, retail: Decimal, shipping: Decimal) -> Product {
        Product {
            id: 1,
            name: "Tee".into(),
            sku: String::new(),
            production_cost: production,
            retail_price: retail,
            shipping_cost: shipping,
            quantity: 1,
            platform: "Stripe".into(),
            notes: String::new(),
        }
    }

    #[test]
    fn stripe_breakdown_matches_hand_math() {
        // $60 retail, $12 production, $4.50 shipping on Stripe:
        // fee = 60*2.9% + 0.30 = 2.04; profit = 41.46; margin = 69.1%.
        let bd = breakdown(&product(dec!(12), dec!(60), dec!(4.50)), &stripe());
        assert_eq!(bd.total_fees, dec!(2.04));
        assert_eq!(bd.profit, dec!(41.46));
        assert_eq!(bd.margin.round_dp(1), dec!(69.1));
    }

    #[test]
    fn zero_retail_is_zero_margin_not_a_panic() {
        let bd = breakdown(&product(dec!(10), dec!(0), dec!(0)), &stripe());
        assert_eq!(bd.margin, dec!(0));
    }

    #[test]
    fn suggest_price_hits_target_margin() {
        // No-fee platform, want 50% margin on $10 cost → price should be $20.
        let free = FeePreset {
            name: "Cash".into(),
            category: Category::Processor,
            lines: vec![FeeLine {
                label: "None".into(),
                percent: dec!(0),
                flat: dec!(0),
                min_price: None,
                max_price: None,
            }],
            note: String::new(),
        };
        let price = suggest_price(dec!(10), dec!(0), &free, dec!(50)).unwrap();
        assert_eq!(price, dec!(20));
        // And feeding it back yields ~50% margin.
        let bd = breakdown(&product(dec!(10), price, dec!(0)), &free);
        assert_eq!(bd.margin.round_dp(1), dec!(50.0));
    }

    #[test]
    fn collection_totals_are_quantity_weighted() {
        use crate::model::Collection;
        // Two of a $60 tee ($12 cost, $4.50 ship) on Stripe.
        // Per unit profit = 41.46; for 2 units profit = 82.92, revenue = 120.
        let mut prod = product(dec!(12), dec!(60), dec!(4.50));
        prod.quantity = 2;
        let coll = Collection {
            id: 1,
            name: "Drop".into(),
            products: vec![prod],
        };
        let t = collection_totals(&coll, std::slice::from_ref(&stripe()));
        assert_eq!(t.units, 2);
        assert_eq!(t.revenue, dec!(120));
        assert_eq!(t.profit, dec!(82.92));
        assert_eq!(t.margin.round_dp(1), dec!(69.1));
    }

    fn poshmark_like() -> FeePreset {
        FeePreset {
            name: "Poshmark".into(),
            category: Category::Marketplace,
            lines: vec![
                crate::model::FeeLine {
                    label: "Commission (<$15)".into(),
                    percent: dec!(0),
                    flat: dec!(2.95),
                    min_price: None,
                    max_price: Some(dec!(15)),
                },
                crate::model::FeeLine {
                    label: "Commission (≥$15)".into(),
                    percent: dec!(20),
                    flat: dec!(0),
                    min_price: Some(dec!(15)),
                    max_price: None,
                },
            ],
            note: String::new(),
        }
    }

    #[test]
    fn poshmark_tiered_fee_switches_at_threshold() {
        let pm = poshmark_like();
        // Under $15: flat $2.95 applies
        let bd_low = breakdown(&product(dec!(3), dec!(10), dec!(0)), &pm);
        assert_eq!(bd_low.total_fees, dec!(2.95));
        // At $15+: 20% applies, not the flat
        let bd_high = breakdown(&product(dec!(3), dec!(20), dec!(0)), &pm);
        assert_eq!(bd_high.total_fees, dec!(4.00));
    }

    #[test]
    fn suggest_price_picks_correct_poshmark_tier() {
        let pm = poshmark_like();
        // High-cost item → should land in ≥$15 tier
        // price = (15 + 2) / (1 - 0.20 - 0.30) = 17 / 0.50 = $34.00
        let p = suggest_price(dec!(15), dec!(2), &pm, dec!(30)).unwrap();
        assert!(p >= dec!(15), "expected ≥$15 tier, got {p}");
        // Low-cost item → should land in <$15 tier
        // price = (3 + 0 + 2.95) / (1 - 0 - 0.30) = 5.95 / 0.70 ≈ $8.50
        let p2 = suggest_price(dec!(3), dec!(0), &pm, dec!(30)).unwrap();
        assert!(p2 < dec!(15), "expected <$15 tier, got {p2}");
    }

    #[test]
    fn suggest_price_unsolvable_returns_none() {
        let free = FeePreset {
            name: "Cash".into(),
            category: Category::Processor,
            lines: vec![FeeLine {
                label: "None".into(),
                percent: dec!(0),
                flat: dec!(0),
                min_price: None,
                max_price: None,
            }],
            note: String::new(),
        };
        // 100% margin is impossible.
        assert!(suggest_price(dec!(10), dec!(0), &free, dec!(100)).is_none());
    }
}
