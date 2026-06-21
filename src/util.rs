//! Small money parsing/formatting helpers shared across the app.

use rust_decimal::Decimal;
use std::str::FromStr;

/// Parse a user-typed money string leniently: strips `$`, commas and spaces.
/// Anything unparseable becomes zero rather than erroring in the user's face.
pub fn parse_money(s: &str) -> Decimal {
    let cleaned = s.trim().trim_start_matches('$').replace(',', "");
    Decimal::from_str(cleaned.trim()).unwrap_or(Decimal::ZERO)
}

/// Format a decimal as money with a given currency symbol. Always shows
/// exactly two decimal places (e.g. `$20.00`, `$16.90`, `-$0.30`).
pub fn money_sym(d: Decimal, sym: &str) -> String {
    let r = d.round_dp(2);
    if r.is_sign_negative() {
        format!("-{}{:.2}", sym, r.abs())
    } else {
        format!("{}{:.2}", sym, r)
    }
}

/// Format a decimal as a plain 2dp string for editable form fields (no `$`).
pub fn money_str(d: Decimal) -> String {
    d.round_dp(2).to_string()
}

/// Format a decimal as a percentage, e.g. `69.1%`.
pub fn percent(d: Decimal) -> String {
    format!("{}%", d.round_dp(1))
}
