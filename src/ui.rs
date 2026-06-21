//! All rendering. Pure draw code — reads `App`, writes to the `Frame`.
//!
//! Layout is: a one-line title bar, a body (three panes, or an empty-state
//! when there's no data yet), and a contextual help/status line. Overlays
//! (forms, pickers, compare, help) are drawn on top in centered popups.

use crate::app::{App, ConfirmAction, FormField, Mode, PresetFormField};
use crate::calc;
use crate::model::Product;
use crate::textfield::TextField;
use crate::util::{parse_money, percent};
use crate::{presets, storage};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Table, Wrap,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

const ACCENT: Color = Color::Rgb(129, 140, 248);
const GREEN: Color = Color::Rgb(52, 211, 153);
const YELLOW: Color = Color::Rgb(251, 191, 36);
const RED: Color = Color::Rgb(248, 113, 113);
const DIM: Color = Color::Rgb(120, 130, 150);
const WHITE: Color = Color::Rgb(226, 232, 240);

/// Color-code a margin: red below 20% (or negative), yellow to 50%, green above.
fn margin_color(m: Decimal) -> Color {
    if m < dec!(20) {
        RED
    } else if m < dec!(50) {
        YELLOW
    } else {
        GREEN
    }
}

/// The ASCII logo, with its `//` comment prefixes stripped.
fn logo_lines() -> Vec<String> {
    include_str!("../logo.txt")
        .lines()
        .map(|l| l.trim_start_matches("//").to_string())
        .filter(|l| !l.trim().is_empty())
        .collect()
}

pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(2), // title bar
        Constraint::Min(0),    // body
        Constraint::Length(1), // status / help
    ])
    .split(area);

    render_title(f, chunks[0]);
    render_body(f, app, chunks[1]);
    render_status(f, app, chunks[2]);

    match app.mode {
        Mode::ProductForm => render_product_form(f, app, area),
        Mode::CollectionForm => render_collection_form(f, app, area),
        Mode::PickPlatform => render_pick_platform(f, app, area),
        Mode::Compare => render_compare(f, app, area),
        Mode::Reverse => render_reverse(f, app, area),
        Mode::ImportPath => render_import(f, app, area),
        Mode::ImportMap => render_import_map(f, app, area),
        Mode::Confirm => render_confirm(f, app, area),
        Mode::Help => render_help(f, area),
        Mode::PresetEditor => render_preset_editor(f, app, area),
        Mode::PresetForm => render_preset_form(f, app, area),
        Mode::MoveProduct => render_move_product(f, app, area),
        Mode::Search => render_search(f, app, area),
        Mode::Settings => render_settings(f, app, area),
        Mode::SaleInput => render_sale_input(f, app, area),
        Mode::WholesaleInput => render_wholesale_input(f, app, area),
        Mode::Browse => {}
    }

    // Error popup renders on top of everything else.
    if app.error_popup.is_some() {
        render_error_popup(f, app, area);
    }
}

fn render_title(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(
            " VORA",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "·Margin ",
            Style::default().fg(WHITE).add_modifier(Modifier::BOLD),
        ),
        Span::styled("— pricing for clothing brands", Style::default().fg(DIM)),
    ]);
    let p = Paragraph::new(line).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(DIM)),
    );
    f.render_widget(p, area);
}

fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![Span::styled(app.status_hint(), Style::default().fg(DIM))];
    if !app.status.is_empty() {
        spans.push(Span::styled(
            format!("   {}", app.status),
            Style::default().fg(GREEN),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_body(f: &mut Frame, app: &mut App, area: Rect) {
    if app.store.collections.is_empty() {
        render_empty_state(f, area);
        return;
    }
    let panes = Layout::horizontal([
        Constraint::Percentage(24),
        Constraint::Percentage(38),
        Constraint::Percentage(38),
    ])
    .split(area);
    render_collections(f, app, panes[0]);
    render_products(f, app, panes[1]);
    render_breakdown(f, app, panes[2]);
}

fn render_empty_state(f: &mut Frame, area: Rect) {
    let mut lines: Vec<Line> = logo_lines()
        .into_iter()
        .map(|l| Line::from(Span::styled(l, Style::default().fg(ACCENT))))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Welcome — let's price your products.",
        Style::default().fg(WHITE).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "Press  n  to create your first collection.",
        Style::default().fg(DIM),
    )));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM));
    let p = Paragraph::new(lines).block(block);
    f.render_widget(p, area);
}

fn render_collections(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == crate::app::Focus::Collections;
    let items: Vec<ListItem> = app
        .store
        .collections
        .iter()
        .map(|c| ListItem::new(format!("{}  ({})", c.name, c.products.len())))
        .collect();
    let border = if focused { ACCENT } else { DIM };
    let list = List::new(items)
        .block(
            Block::bordered()
                .title(" Collections ")
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(border)),
        )
        .highlight_style(
            Style::default()
                .fg(if focused { ACCENT } else { WHITE })
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.coll_state);
}

fn render_products(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == crate::app::Focus::Products;
    let ci = app.coll_state.selected();

    // Build display list in sort order.
    let sorted = app.sorted_indices();
    let items: Vec<ListItem> = ci
        .and_then(|i| app.store.collections.get(i))
        .map(|c| {
            sorted
                .iter()
                .filter_map(|&si| c.products.get(si))
                .map(|p| {
                    let margin = app
                        .preset_by_name(&p.platform)
                        .map(|preset| calc::breakdown(p, preset).margin);
                    // Right column: fixed 14 chars so price+margin are always
                    // right-justified and rows align even at different pane widths.
                    // Left column fills whatever space remains.
                    const RIGHT_W: usize = 14; // "  $999.99 99.9%"
                    let inner_w = (area.width as usize).saturating_sub(4); // borders + highlight symbol
                    let name_w = inner_w.saturating_sub(RIGHT_W).max(4);
                    let right = match margin {
                        Some(m) => format!("{:>7}  {:>5}", app.money(p.retail_price), percent(m)),
                        None => format!("{:>14}", app.money(p.retail_price)),
                    };
                    let color = margin.map(margin_color).unwrap_or(WHITE);
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("{:<width$}", truncate(&p.name, name_w), width = name_w),
                            Style::default().fg(WHITE),
                        ),
                        Span::styled(right, Style::default().fg(color)),
                    ]))
                })
                .collect()
        })
        .unwrap_or_default();

    let sort_label = app.sort_label();
    let mut title = ci
        .and_then(|i| app.store.collections.get(i))
        .map(|c| format!(" {}{} ", c.name, sort_label))
        .unwrap_or_else(|| " Products ".to_string());
    if let Some(pct) = app.active_sale_pct {
        title.push_str(&format!("[-{:.0}%] ", pct));
    }
    if let Some(pct) = app.active_wholesale_pct {
        title.push_str(&format!("[WS {:.0}%] ", pct));
    }
    let border = if focused { ACCENT } else { DIM };

    if items.is_empty() {
        let block = Block::bordered()
            .title(title)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border));
        let hint = Paragraph::new(Line::from(Span::styled(
            "No products yet — press n to add one.",
            Style::default().fg(DIM),
        )))
        .block(block)
        .wrap(Wrap { trim: true });
        f.render_widget(hint, area);
        return;
    }

    let list = List::new(items)
        .block(
            Block::bordered()
                .title(title)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(border)),
        )
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .bg(Color::Rgb(40, 44, 60)),
        )
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.prod_state);
}

fn render_breakdown(f: &mut Frame, app: &App, area: Rect) {
    // The right pane is contextual: collection totals when the left pane is
    // focused, otherwise the selected product's per-unit margin breakdown.
    if app.focus == crate::app::Focus::Collections {
        render_collection_totals(f, app, area);
    } else {
        render_product_breakdown(f, app, area);
    }
}

fn render_product_breakdown(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered()
        .title(" Breakdown ")
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM));

    let product = app.current_product();
    let preset = product.and_then(|p| app.preset_by_name(&p.platform));

    let lines: Vec<Line> = match (product, preset) {
        (Some(p), Some(preset)) => {
            let bd = calc::breakdown(p, preset);
            let mut out = vec![
                Line::from(vec![
                    Span::styled("Platform  ", Style::default().fg(DIM)),
                    Span::styled(
                        preset.name.clone(),
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(Span::styled(preset.note.clone(), Style::default().fg(DIM))),
                Line::from(""),
                kv("Production", app.money(bd.production), WHITE),
                kv("Shipping", app.money(bd.shipping), WHITE),
            ];
            for (label, amt) in &bd.fee_lines {
                out.push(kv(label, app.money(*amt), YELLOW));
            }
            out.push(Line::from(Span::styled(
                "──────────────────────────",
                Style::default().fg(DIM),
            )));
            out.push(kv("Retail", app.money(bd.retail), WHITE));
            out.push(kv("Total fees", app.money(bd.total_fees), YELLOW));
            let mc = margin_color(bd.margin);
            out.push(Line::from(vec![
                Span::styled(format!("{:<14}", "Profit/unit"), Style::default().fg(DIM)),
                Span::styled(
                    app.money(bd.profit),
                    Style::default().fg(mc).add_modifier(Modifier::BOLD),
                ),
            ]));
            out.push(Line::from(vec![
                Span::styled(format!("{:<14}", "Margin"), Style::default().fg(DIM)),
                Span::styled(
                    percent(bd.margin),
                    Style::default().fg(mc).add_modifier(Modifier::BOLD),
                ),
            ]));
            if p.quantity > 1 {
                let batch = bd.profit * Decimal::from(p.quantity);
                out.push(Line::from(""));
                out.push(kv(&format!("Batch ×{}", p.quantity), app.money(batch), mc));
            }

            // Minimum price warning — shown whenever retail is set but profit ≤ 0.
            if !p.retail_price.is_zero() && bd.profit <= Decimal::ZERO {
                let min_price = calc::suggest_price(
                    p.production_cost,
                    p.shipping_cost,
                    preset,
                    Decimal::ZERO,
                );
                out.push(Line::from(""));
                let warn_text = match min_price {
                    Some(mp) => format!("⚠  Below break-even  ·  min: {}", app.money(mp)),
                    None => "⚠  Below break-even  ·  not achievable on this platform".into(),
                };
                out.push(Line::from(Span::styled(
                    warn_text,
                    Style::default().fg(RED).add_modifier(Modifier::BOLD),
                )));
            }

            // Sale scenario section.
            if let Some(sale_pct) = app.active_sale_pct {
                let sale_price =
                    (p.retail_price * (dec!(100) - sale_pct) / dec!(100)).round_dp(2);
                let (_, sale_fees) = calc::fees_for(sale_price, preset);
                let sale_profit =
                    sale_price - p.production_cost - p.shipping_cost - sale_fees;
                let sale_margin = if sale_price.is_zero() {
                    Decimal::ZERO
                } else {
                    sale_profit / sale_price * dec!(100)
                };
                let smc = margin_color(sale_margin);
                out.push(Line::from(""));
                out.push(Line::from(Span::styled(
                    format!("── Sale -{:.0}% ─────────────────", sale_pct),
                    Style::default().fg(ACCENT),
                )));
                out.push(kv("Sale price", app.money(sale_price), WHITE));
                out.push(kv("Fees", app.money(sale_fees), YELLOW));
                out.push(Line::from(vec![
                    Span::styled(format!("{:<14}", "Profit/unit"), Style::default().fg(DIM)),
                    Span::styled(
                        app.money(sale_profit),
                        Style::default().fg(smc).add_modifier(Modifier::BOLD),
                    ),
                ]));
                out.push(Line::from(vec![
                    Span::styled(format!("{:<14}", "Margin"), Style::default().fg(DIM)),
                    Span::styled(
                        percent(sale_margin),
                        Style::default().fg(smc).add_modifier(Modifier::BOLD),
                    ),
                ]));
            }

            // Wholesale section (no platform fees — direct B2B invoice).
            if let Some(ws_pct) = app.active_wholesale_pct {
                let ws_price =
                    (p.retail_price * ws_pct / dec!(100)).round_dp(2);
                let ws_profit = ws_price - p.production_cost - p.shipping_cost;
                let ws_margin = if ws_price.is_zero() {
                    Decimal::ZERO
                } else {
                    ws_profit / ws_price * dec!(100)
                };
                let wmc = margin_color(ws_margin);
                out.push(Line::from(""));
                out.push(Line::from(Span::styled(
                    format!("── Wholesale {:.0}% ──────────────", ws_pct),
                    Style::default().fg(ACCENT),
                )));
                out.push(kv("Wholesale $", app.money(ws_price), WHITE));
                out.push(Line::from(vec![
                    Span::styled(format!("{:<14}", "Profit"), Style::default().fg(DIM)),
                    Span::styled(
                        app.money(ws_profit),
                        Style::default().fg(wmc).add_modifier(Modifier::BOLD),
                    ),
                ]));
                out.push(Line::from(vec![
                    Span::styled(format!("{:<14}", "Margin"), Style::default().fg(DIM)),
                    Span::styled(
                        percent(ws_margin),
                        Style::default().fg(wmc).add_modifier(Modifier::BOLD),
                    ),
                ]));
                out.push(Line::from(Span::styled(
                    "no fees — direct invoice",
                    Style::default().fg(DIM),
                )));
            }

            if !p.notes.is_empty() {
                out.push(Line::from(""));
                out.push(Line::from(vec![
                    Span::styled(format!("{:<14}", "Notes"), Style::default().fg(DIM)),
                    Span::styled(p.notes.clone(), Style::default().fg(WHITE)),
                ]));
            }
            out
        }
        _ => vec![Line::from(Span::styled(
            "Select a product to see its margin.",
            Style::default().fg(DIM),
        ))],
    };

    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn render_collection_totals(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::bordered()
        .title(" Collection totals ")
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM));

    let lines: Vec<Line> = match app.current_collection() {
        Some(coll) => {
            let t = calc::collection_totals(coll, &app.presets);
            let mc = margin_color(t.margin);
            vec![
                Line::from(Span::styled(
                    coll.name.clone(),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!("{} product(s) · {} unit(s)", t.products, t.units),
                    Style::default().fg(DIM),
                )),
                Line::from(Span::styled(
                    "if every unit sells:",
                    Style::default().fg(DIM),
                )),
                Line::from(""),
                kv("Revenue", app.money(t.revenue), WHITE),
                kv("Cost of goods", app.money(t.cost_of_goods), YELLOW),
                kv("Shipping", app.money(t.shipping), YELLOW),
                kv("Fees", app.money(t.fees), YELLOW),
                Line::from(Span::styled(
                    "──────────────────────────",
                    Style::default().fg(DIM),
                )),
                Line::from(vec![
                    Span::styled(format!("{:<14}", "Profit"), Style::default().fg(DIM)),
                    Span::styled(
                        app.money(t.profit),
                        Style::default().fg(mc).add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(format!("{:<14}", "Blended"), Style::default().fg(DIM)),
                    Span::styled(
                        percent(t.margin),
                        Style::default().fg(mc).add_modifier(Modifier::BOLD),
                    ),
                ]),
            ]
        }
        None => vec![Line::from(Span::styled(
            "Select a collection.",
            Style::default().fg(DIM),
        ))],
    };

    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

/// A "label........value" line with a fixed-width label.
fn kv(label: &str, value: String, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{:<14}", label), Style::default().fg(DIM)),
        Span::styled(value, Style::default().fg(color)),
    ])
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

// ---- Overlays -----------------------------------------------------------

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(v[1])[1]
}

/// Render one form field as a line, drawing a block cursor when it's active.
fn field_line(label: &str, tf: &TextField, active: bool) -> Line<'static> {
    let label_color = if active { ACCENT } else { DIM };
    let mut spans = vec![Span::styled(
        format!("{:<13}", label),
        Style::default().fg(label_color),
    )];
    let chars: Vec<char> = tf.value().chars().collect();
    if active {
        let cur = tf.cursor();
        for (i, ch) in chars.iter().enumerate() {
            if i == cur {
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default().bg(ACCENT).fg(Color::Black),
                ));
            } else {
                spans.push(Span::styled(ch.to_string(), Style::default().fg(WHITE)));
            }
        }
        if cur >= chars.len() {
            spans.push(Span::styled(" ", Style::default().bg(ACCENT)));
        }
    } else {
        let shown = if chars.is_empty() {
            "—".to_string()
        } else {
            tf.value().to_string()
        };
        spans.push(Span::styled(shown, Style::default().fg(WHITE)));
    }
    Line::from(spans)
}

/// A ◀ value ▶ selector line (used for the cost-mode and platform fields).
fn toggle_line(label: &str, value: &str, active: bool) -> Line<'static> {
    let color = if active { ACCENT } else { DIM };
    Line::from(vec![
        Span::styled(format!("{:<13}", label), Style::default().fg(color)),
        Span::styled("◀ ", Style::default().fg(color)),
        Span::styled(
            value.to_string(),
            Style::default().fg(WHITE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ▶", Style::default().fg(color)),
    ])
}

fn render_product_form(f: &mut Frame, app: &App, area: Rect) {
    let form = match app.form.as_ref() {
        Some(f) => f,
        None => return,
    };
    let popup = centered_rect(64, 90, area);
    f.render_widget(Clear, popup);

    let title = if form.editing.is_some() {
        " Edit product "
    } else {
        " New product "
    };
    let block = Block::bordered()
        .title(title)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let cost_label = if form.cost_mode {
        "Batch total $"
    } else {
        "Production $"
    };

    let preset = app.presets.get(form.platform_idx);
    let pname = preset.map(|p| p.name.as_str()).unwrap_or("—");

    let mut lines = vec![
        field_line("Name", &form.name, form.field == FormField::Name),
        field_line("SKU", &form.sku, form.field == FormField::Sku),
        field_line("Note", &form.note, form.field == FormField::Note),
        toggle_line(
            "Cost mode",
            if form.cost_mode {
                "Total batch"
            } else {
                "Per-unit"
            },
            form.field == FormField::CostMode,
        ),
        field_line(
            cost_label,
            &form.production,
            form.field == FormField::Production,
        ),
        field_line(
            "Quantity",
            &form.quantity,
            form.field == FormField::Quantity,
        ),
        field_line("Retail $", &form.retail, form.field == FormField::Retail),
        field_line(
            "Shipping $",
            &form.shipping,
            form.field == FormField::Shipping,
        ),
        toggle_line("Platform", pname, form.field == FormField::Platform),
    ];

    // Live, bulk-aware preview from the values typed so far.
    lines.push(Line::from(""));
    if let Some(preset) = preset {
        let unit_cost = form.unit_cost();
        let temp = Product {
            id: 0,
            name: String::new(),
            sku: String::new(),
            production_cost: unit_cost,
            retail_price: parse_money(form.retail.value()),
            shipping_cost: parse_money(form.shipping.value()),
            quantity: form.qty(),
            platform: pname.to_string(),
            notes: String::new(),
        };
        let bd = calc::breakdown(&temp, preset);
        let mc = margin_color(bd.margin);
        if form.cost_mode {
            lines.push(Line::from(Span::styled(
                format!("Per-unit cost {} (÷ {})", app.money(unit_cost), form.qty()),
                Style::default().fg(DIM),
            )));
        }
        lines.push(Line::from(vec![
            Span::styled("Per unit   ", Style::default().fg(DIM)),
            Span::styled(
                format!("Profit {}   ", app.money(bd.profit)),
                Style::default().fg(mc),
            ),
            Span::styled(
                format!("Margin {}", percent(bd.margin)),
                Style::default().fg(mc).add_modifier(Modifier::BOLD),
            ),
        ]));
        if form.qty() > 1 {
            let batch = bd.profit * Decimal::from(form.qty());
            lines.push(Line::from(Span::styled(
                format!("Batch profit (×{}) {}", form.qty(), app.money(batch)),
                Style::default().fg(mc),
            )));
        }
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_collection_form(f: &mut Frame, app: &App, area: Rect) {
    let form = match app.coll_form.as_ref() {
        Some(f) => f,
        None => return,
    };
    let popup = centered_rect(50, 18, area);
    f.render_widget(Clear, popup);
    let title = if form.editing.is_some() {
        " Rename collection "
    } else {
        " New collection "
    };
    let block = Block::bordered()
        .title(title)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let lines = vec![
        field_line("Name", &form.name, true),
        Line::from(""),
        Line::from(Span::styled(
            "e.g. \"SS26 Drop\", \"Basics\", \"Hoodies\"",
            Style::default().fg(DIM),
        )),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_pick_platform(f: &mut Frame, app: &mut App, area: Rect) {
    let popup = centered_rect(64, 70, area);
    f.render_widget(Clear, popup);
    let items: Vec<ListItem> = app
        .presets
        .iter()
        .map(|p| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<24}", truncate(&p.name, 24)),
                    Style::default().fg(WHITE),
                ),
                Span::styled(
                    format!("[{}] ", p.category.label()),
                    Style::default().fg(ACCENT),
                ),
                Span::styled(p.note.clone(), Style::default().fg(DIM)),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::bordered()
                .title(" Choose platform ")
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ACCENT)),
        )
        .highlight_style(
            Style::default()
                .bg(ACCENT)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, popup, &mut app.pick_state);
}

fn render_compare(f: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(82, 80, area);
    f.render_widget(Clear, popup);

    let product = match app.current_product() {
        Some(p) => p,
        None => return,
    };

    let title = format!(
        " Compare — {} @ {} ",
        product.name,
        app.money(product.retail_price)
    );
    let block = Block::bordered()
        .title(title)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    // Compute margin for this product on every preset, best margin first.
    let mut rows: Vec<(String, String, Decimal, Decimal)> = app
        .presets
        .iter()
        .map(|preset| {
            let bd = calc::breakdown(product, preset);
            (
                preset.name.clone(),
                app.money(bd.total_fees),
                bd.profit,
                bd.margin,
            )
        })
        .collect();
    rows.sort_by(|a, b| b.3.cmp(&a.3));

    let table_rows: Vec<Row> = rows
        .iter()
        .map(|(name, fees, profit, margin)| {
            let mc = margin_color(*margin);
            Row::new(vec![
                Cell::from(name.clone()),
                Cell::from(fees.clone()),
                Cell::from(app.money(*profit)),
                Cell::from(Span::styled(
                    percent(*margin),
                    Style::default().fg(mc).add_modifier(Modifier::BOLD),
                )),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(46),
        Constraint::Percentage(18),
        Constraint::Percentage(18),
        Constraint::Percentage(18),
    ];
    let table = Table::new(table_rows, widths)
        .header(
            Row::new(vec!["Platform", "Fees", "Profit", "Margin"])
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    f.render_widget(table, inner);
}

fn render_reverse(f: &mut Frame, app: &App, area: Rect) {
    let reverse = match app.reverse.as_ref() {
        Some(r) => r,
        None => return,
    };
    let product = app
        .store
        .collections
        .iter()
        .flat_map(|c| &c.products)
        .find(|p| p.id == reverse.product_id);

    let popup = centered_rect(60, 64, area);
    f.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(" Reverse pricing ")
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines = Vec::new();
    if let Some(p) = product {
        lines.push(Line::from(Span::styled(
            p.name.clone(),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            format!("via {}", p.platform),
            Style::default().fg(DIM),
        )));
        lines.push(Line::from(""));
        lines.push(kv("Production", app.money(p.production_cost), WHITE));
        lines.push(kv("Shipping", app.money(p.shipping_cost), WHITE));
        lines.push(kv("Current price", app.money(p.retail_price), WHITE));
        lines.push(Line::from(""));
        lines.push(field_line("Target %", &reverse.target_margin, true));
        lines.push(Line::from(""));
        match app.reverse_suggestion() {
            Some(price) => {
                lines.push(Line::from(vec![
                    Span::styled("Suggested price  ", Style::default().fg(DIM)),
                    Span::styled(
                        app.money(price),
                        Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
                    ),
                ]));
                lines.push(Line::from(Span::styled(
                    "Enter applies it as the new retail price.",
                    Style::default().fg(DIM),
                )));
            }
            None => lines.push(Line::from(Span::styled(
                "That margin isn't reachable with these fees.",
                Style::default().fg(RED),
            ))),
        }
    } else {
        lines.push(Line::from(Span::styled(
            "Product not found.",
            Style::default().fg(RED),
        )));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_import(f: &mut Frame, app: &App, area: Rect) {
    let field = match app.import_path.as_ref() {
        Some(t) => t,
        None => return,
    };
    let popup = centered_rect(74, 44, area);
    f.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(" Import Spreadsheet ")
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let lines = vec![
        Line::from(Span::styled(
            "Path to a .xlsx file to import:",
            Style::default().fg(DIM),
        )),
        Line::from(""),
        field_line("Path", field, true),
        Line::from(""),
        Line::from(Span::styled(
            "Works with any layout — you'll match the columns on the next screen.",
            Style::default().fg(DIM),
        )),
        Line::from(Span::styled(
            "Tip: you can paste a quoted path (\"...\") straight from \"Copy as path\".",
            Style::default().fg(DIM),
        )),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_import_map(f: &mut Frame, app: &App, area: Rect) {
    let state = match app.import_map.as_ref() {
        Some(s) => s,
        None => return,
    };
    let sheet = &state.sheets[0];

    let popup = centered_rect(78, 86, area);
    f.render_widget(Clear, popup);
    let title = if state.sheets.len() > 1 {
        format!(
            " Map Columns — {} (+{} more sheets) ",
            sheet.name,
            state.sheets.len() - 1
        )
    } else {
        format!(" Map Columns — {} ", sheet.name)
    };
    let block = Block::bordered()
        .title(title)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let red = Color::Rgb(248, 113, 113);
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "Match each VORA field to a column from your spreadsheet.",
        Style::default().fg(DIM),
    )));
    lines.push(Line::from(Span::styled(
        "←/→ pick column · ↑/↓ move · Enter import · Esc cancel    (* required)",
        Style::default().fg(DIM),
    )));
    lines.push(Line::from(""));

    let fields = crate::xlsx_import::ImportField::ALL;
    for (i, field) in fields.iter().enumerate() {
        let active = state.cursor == i;
        let col = state.mapping.get(*field);
        let required_unmapped = field.required() && col.is_none();

        let label_txt = if field.required() {
            format!("{} *", field.label())
        } else {
            field.label().to_string()
        };
        let assigned = match col {
            Some(c) => format!("[{}] {}", c + 1, sheet.header_label(c)),
            None => "(skip)".to_string(),
        };

        let label_color = if active {
            ACCENT
        } else if required_unmapped {
            red
        } else {
            DIM
        };
        let val_color = if required_unmapped { red } else { WHITE };
        let arrow_color = if active { ACCENT } else { DIM };
        let val_mod = if active {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };

        let mut spans = vec![
            Span::styled(format!("{:<13}", label_txt), Style::default().fg(label_color)),
            Span::styled("◀ ", Style::default().fg(arrow_color)),
            Span::styled(assigned, Style::default().fg(val_color).add_modifier(val_mod)),
            Span::styled(" ▶", Style::default().fg(arrow_color)),
        ];
        let preview = col.and_then(|c| sheet.sample.get(c)).cloned().unwrap_or_default();
        if !preview.is_empty() {
            spans.push(Span::styled(
                format!("   e.g. {}", truncate(&preview, 24)),
                Style::default().fg(DIM),
            ));
        }
        lines.push(Line::from(spans));
    }

    // Platform picker — applied to every imported product.
    lines.push(Line::from(""));
    let plat_row = fields.len();
    let plat_name = app
        .presets
        .get(state.platform_idx)
        .map(|p| p.name.as_str())
        .unwrap_or("—");
    lines.push(toggle_line("Platform", plat_name, state.cursor == plat_row));

    // Collection name(s) — one editable field per sheet.
    lines.push(Line::from(""));
    let single = state.coll_names.len() == 1;
    for (si, tf) in state.coll_names.iter().enumerate() {
        let active = state.cursor == plat_row + 1 + si;
        let label = if single {
            "Collection".to_string()
        } else {
            truncate(&state.sheets[si].name, 11)
        };
        lines.push(field_line(&label, tf, active));
    }

    // Summary + live preview of the first product as it will import.
    if let Some(prev) = app.import_preview() {
        let green = Color::Rgb(52, 211, 153);
        lines.push(Line::from(""));
        let mut summary = vec![Span::styled(
            format!("{} product(s) ready", prev.valid),
            Style::default().fg(green).add_modifier(Modifier::BOLD),
        )];
        if prev.skipped > 0 {
            summary.push(Span::styled(
                format!("    ·    {} skipped (no valid price)", prev.skipped),
                Style::default().fg(red),
            ));
        }
        lines.push(Line::from(summary));

        if let Some((p, preset)) = &prev.first {
            let bd = calc::breakdown(p, preset);
            let mc = margin_color(bd.margin);
            lines.push(Line::from(vec![
                Span::styled("Preview  ", Style::default().fg(DIM)),
                Span::styled(truncate(&p.name, 22), Style::default().fg(WHITE)),
                Span::styled("  retail ", Style::default().fg(DIM)),
                Span::styled(app.money(p.retail_price), Style::default().fg(WHITE)),
                Span::styled("  fees ", Style::default().fg(DIM)),
                Span::styled(app.money(bd.total_fees), Style::default().fg(WHITE)),
                Span::styled("  profit ", Style::default().fg(DIM)),
                Span::styled(app.money(bd.profit), Style::default().fg(mc)),
                Span::styled("  margin ", Style::default().fg(DIM)),
                Span::styled(percent(bd.margin), Style::default().fg(mc)),
            ]));
        }
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn render_error_popup(f: &mut Frame, app: &App, area: Rect) {
    let msg = match app.error_popup.as_deref() {
        Some(s) => s,
        None => return,
    };
    let popup = centered_rect(64, 36, area);
    f.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(" Error ")
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(248, 113, 113))); // red
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines: Vec<Line> = msg
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(WHITE))))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press any key to dismiss.",
        Style::default().fg(DIM),
    )));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn render_confirm(f: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(50, 18, area);
    f.render_widget(Clear, popup);
    let msg = match &app.confirm {
        Some(ConfirmAction::DeleteCollection(_)) => {
            "Delete this collection and ALL its products?".to_string()
        }
        Some(ConfirmAction::DeleteProduct(_)) => "Delete this product?".to_string(),
        Some(ConfirmAction::DeletePreset(name)) => format!("Delete preset \"{name}\"?"),
        None => String::new(),
    };
    let block = Block::bordered()
        .title(" Confirm ")
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(RED));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let lines = vec![
        Line::from(Span::styled(msg, Style::default().fg(WHITE))),
        Line::from(""),
        Line::from(vec![
            Span::styled("y", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)),
            Span::styled(" yes    ", Style::default().fg(DIM)),
            Span::styled("n", Style::default().fg(RED).add_modifier(Modifier::BOLD)),
            Span::styled(" no", Style::default().fg(DIM)),
        ]),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

// ---- Custom preset editor -----------------------------------------------

/// Render a TextField inline (within a larger Line) with cursor highlight.
fn cursor_spans(tf: &TextField, active: bool) -> Vec<Span<'static>> {
    let chars: Vec<char> = tf.value().chars().collect();
    let mut spans = Vec::new();
    if active {
        let cur = tf.cursor();
        for (i, ch) in chars.iter().enumerate() {
            spans.push(if i == cur {
                Span::styled(ch.to_string(), Style::default().bg(ACCENT).fg(Color::Black))
            } else {
                Span::styled(ch.to_string(), Style::default().fg(WHITE))
            });
        }
        if cur >= chars.len() {
            spans.push(Span::styled(" ".to_string(), Style::default().bg(ACCENT)));
        }
    } else {
        let shown = if chars.is_empty() {
            "—".to_string()
        } else {
            tf.value().to_string()
        };
        spans.push(Span::styled(shown, Style::default().fg(WHITE)));
    }
    spans
}

/// One fee-line row: "  N. Label [...]   % [...]   +$ [...]"
fn preset_fee_row(
    n: usize,
    pl: &crate::app::PresetLineForm,
    la: bool,
    pa: bool,
    fa: bool,
) -> Line<'static> {
    let lc = if la { ACCENT } else { DIM };
    let pc = if pa { ACCENT } else { DIM };
    let fc = if fa { ACCENT } else { DIM };

    let mut spans = vec![
        Span::styled(format!("  {:>2}. ", n + 1), Style::default().fg(DIM)),
        Span::styled("Label ", Style::default().fg(lc)),
    ];
    spans.extend(cursor_spans(&pl.label, la));
    spans.push(Span::styled("   % ".to_string(), Style::default().fg(pc)));
    spans.extend(cursor_spans(&pl.percent, pa));
    spans.push(Span::styled("   +$ ".to_string(), Style::default().fg(fc)));
    spans.extend(cursor_spans(&pl.flat, fa));
    Line::from(spans)
}

fn render_preset_editor(f: &mut Frame, app: &mut App, area: Rect) {
    let popup = centered_rect(72, 78, area);
    f.render_widget(Clear, popup);

    let items: Vec<ListItem> = if app.store.custom_presets.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No custom presets yet — press  n  to create one.",
            Style::default().fg(DIM),
        )))]
    } else {
        app.store
            .custom_presets
            .iter()
            .map(|p| {
                let fee_summary = p
                    .lines
                    .iter()
                    .map(|l| {
                        if l.percent.is_zero() {
                            format!("+${}", l.flat)
                        } else if l.flat.is_zero() {
                            format!("{}%", l.percent)
                        } else {
                            format!("{}%+${}", l.percent, l.flat)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("  ");
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:<26}", truncate(&p.name, 26)),
                        Style::default().fg(WHITE),
                    ),
                    Span::styled(fee_summary, Style::default().fg(DIM)),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::bordered()
                .title(" Custom fee presets  (n new · e edit · d delete · Esc close) ")
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ACCENT)),
        )
        .highlight_style(
            Style::default()
                .bg(ACCENT)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    if app.store.custom_presets.is_empty() {
        f.render_widget(list, popup);
    } else {
        f.render_stateful_widget(list, popup, &mut app.preset_editor_state);
    }
}

fn render_preset_form(f: &mut Frame, app: &App, area: Rect) {
    let form = match app.preset_form.as_ref() {
        Some(f) => f,
        None => return,
    };

    let popup = centered_rect(70, 90, area);
    f.render_widget(Clear, popup);

    let title = if form.editing.is_some() {
        " Edit preset "
    } else {
        " New preset "
    };
    let block = Block::bordered()
        .title(title)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let name_active = matches!(form.field, PresetFormField::Name);
    let note_active = matches!(form.field, PresetFormField::Note);

    let mut lines = vec![
        field_line("Name", &form.name, name_active),
        field_line("Note", &form.note, note_active),
        Line::from(""),
        Line::from(Span::styled(
            "Fee lines (Ctrl+N add · Ctrl+D remove last):",
            Style::default().fg(DIM),
        )),
        Line::from(Span::styled(
            "  #    Label               %       +$ flat",
            Style::default().fg(DIM),
        )),
    ];

    for (i, pline) in form.lines.iter().enumerate() {
        let la = matches!(form.field, PresetFormField::LineLabel(j) if j == i);
        let pa = matches!(form.field, PresetFormField::LinePercent(j) if j == i);
        let fa = matches!(form.field, PresetFormField::LineFlat(j) if j == i);
        lines.push(preset_fee_row(i, pline, la, pa, fa));
    }

    // Live fee-on-$50-sale preview.
    lines.push(Line::from(""));
    if !form.name.value().trim().is_empty() {
        let sample_lines: Vec<crate::model::FeeLine> = form
            .lines
            .iter()
            .map(|l| crate::model::FeeLine {
                label: l.label.value().trim().to_string(),
                percent: parse_money(l.percent.value()),
                flat: parse_money(l.flat.value()),
                min_price: None,
                max_price: None,
            })
            .collect();
        let sample_preset = crate::model::FeePreset {
            name: form.name.value().trim().to_string(),
            category: crate::model::Category::Custom,
            lines: sample_lines,
            note: String::new(),
        };
        let (_, total) = calc::fees_for(dec!(50), &sample_preset);
        lines.push(Line::from(vec![
            Span::styled("Preview on $50 sale: ", Style::default().fg(DIM)),
            Span::styled(app.money(total), Style::default().fg(YELLOW)),
            Span::styled(" in fees", Style::default().fg(DIM)),
        ]));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_sale_input(f: &mut Frame, app: &App, area: Rect) {
    let sale_tf = match app.sale_field.as_ref() {
        Some(t) => t,
        None => return,
    };
    let popup = centered_rect(58, 68, area);
    f.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(" Sale tools  (Tab switch · Enter apply · Esc cancel) ")
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let discount_active = !app.sale_input_focus;
    let fake_active = app.sale_input_focus;
    let discount_pct = parse_money(sale_tf.value());

    // Live fake price calculation.
    let fake_tf = app.fake_target_field.as_ref();
    let fake_target = fake_tf
        .map(|t| parse_money(t.value()))
        .unwrap_or(Decimal::ZERO);
    let fake_list_price = if discount_pct > Decimal::ZERO
        && discount_pct < dec!(100)
        && !fake_target.is_zero()
    {
        Some((fake_target / (dec!(100) - discount_pct) * dec!(100)).round_dp(2))
    } else {
        None
    };

    let mut lines = vec![
        Line::from(Span::styled(
            "── Real sale scenario ──────────────────────",
            Style::default().fg(ACCENT),
        )),
        Line::from(Span::styled(
            "See how your margins change at a lower price.",
            Style::default().fg(DIM),
        )),
        Line::from(""),
        field_line("Discount %", sale_tf, discount_active),
        Line::from(Span::styled(
            "Shows updated margins in the breakdown pane.",
            Style::default().fg(DIM),
        )),
        Line::from(Span::styled(
            "Press % again from browse to clear.",
            Style::default().fg(DIM),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "── Fake sale calculator ────────────────────",
            Style::default().fg(ACCENT),
        )),
        Line::from(Span::styled(
            "Find the listing price that lands at your",
            Style::default().fg(DIM),
        )),
        Line::from(Span::styled(
            "desired price after showing a discount.",
            Style::default().fg(DIM),
        )),
        Line::from(Span::styled(
            "Great for Etsy where buyers love a sale.",
            Style::default().fg(DIM),
        )),
        Line::from(""),
    ];

    if let Some(fake_tf) = fake_tf {
        lines.push(field_line("Desired price", fake_tf, fake_active));
    }

    match fake_list_price {
        Some(lp) => {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("List at  ", Style::default().fg(DIM)),
                Span::styled(
                    app.money(lp),
                    Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  to show -{:.0}% and receive {}", discount_pct, app.money(fake_target)),
                    Style::default().fg(DIM),
                ),
            ]));
        }
        None => {
            lines.push(Line::from(Span::styled(
                "Enter a discount % and desired price above.",
                Style::default().fg(DIM),
            )));
        }
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn render_wholesale_input(f: &mut Frame, app: &App, area: Rect) {
    let tf = match app.wholesale_field.as_ref() {
        Some(t) => t,
        None => return,
    };
    let popup = centered_rect(50, 24, area);
    f.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(" Wholesale view  (Enter apply · Esc cancel) ")
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let lines = vec![
        field_line("% of retail", tf, true),
        Line::from(""),
        Line::from(Span::styled(
            "e.g. 50 = boutique buys at half your",
            Style::default().fg(DIM),
        )),
        Line::from(Span::styled(
            "retail price. Assumes direct invoice",
            Style::default().fg(DIM),
        )),
        Line::from(Span::styled(
            "(no platform fees). Press w to clear.",
            Style::default().fg(DIM),
        )),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_move_product(f: &mut Frame, app: &mut App, area: Rect) {
    let popup = centered_rect(60, 60, area);
    f.render_widget(Clear, popup);

    let current_ci = app.coll_state.selected().unwrap_or(0);
    let items: Vec<ListItem> = app
        .store
        .collections
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if i == current_ci {
                ListItem::new(Line::from(vec![
                    Span::styled(c.name.clone(), Style::default().fg(DIM)),
                    Span::styled(" (current)", Style::default().fg(DIM)),
                ]))
            } else {
                ListItem::new(Span::styled(c.name.clone(), Style::default().fg(WHITE)))
            }
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::bordered()
                .title(" Move to collection  (Enter confirm · Esc cancel) ")
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(ACCENT)),
        )
        .highlight_style(
            Style::default()
                .bg(ACCENT)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, popup, &mut app.move_list_state);
}

fn render_search(f: &mut Frame, app: &mut App, area: Rect) {
    let popup = centered_rect(70, 80, area);
    f.render_widget(Clear, popup);

    let block = Block::bordered()
        .title(" Search  (Enter jump · Esc close) ")
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    if let Some(tf) = app.search_query.as_ref() {
        f.render_widget(Paragraph::new(field_line("Search", tf, true)), chunks[0]);
    }

    let no_query = app
        .search_query
        .as_ref()
        .map(|t| t.value().is_empty())
        .unwrap_or(true);
    let items: Vec<ListItem> = if app.search_results.is_empty() {
        let hint = if no_query {
            "Type to search across all products…"
        } else {
            "No matches."
        };
        vec![ListItem::new(Span::styled(
            hint,
            Style::default().fg(DIM),
        ))]
    } else {
        app.search_results
            .iter()
            .map(|&(ci, si)| {
                let coll_name = app
                    .store
                    .collections
                    .get(ci)
                    .map(|c| c.name.as_str())
                    .unwrap_or("?");
                let prod = app
                    .store
                    .collections
                    .get(ci)
                    .and_then(|c| c.products.get(si));
                match prod {
                    Some(p) => ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("{:<22}", truncate(&p.name, 22)),
                            Style::default().fg(WHITE),
                        ),
                        Span::styled(
                            format!("{:<16}", truncate(coll_name, 16)),
                            Style::default().fg(ACCENT),
                        ),
                        Span::styled(app.money(p.retail_price), Style::default().fg(DIM)),
                    ])),
                    None => ListItem::new(Span::raw("")),
                }
            })
            .collect()
    };

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(ACCENT)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, chunks[2], &mut app.search_result_state);
}

fn render_settings(f: &mut Frame, app: &App, area: Rect) {
    let tf = match app.settings_field.as_ref() {
        Some(t) => t,
        None => return,
    };
    let popup = centered_rect(50, 24, area);
    f.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(" Settings  (Enter save · Esc cancel) ")
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let lines = vec![
        field_line("Currency", tf, true),
        Line::from(""),
        Line::from(Span::styled(
            "Symbol shown before all prices.",
            Style::default().fg(DIM),
        )),
        Line::from(Span::styled(
            "e.g.  $   £   €   ¥",
            Style::default().fg(DIM),
        )),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_help(f: &mut Frame, area: Rect) {
    let popup = centered_rect(82, 92, area);
    f.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(" Help — VORA·Margin ")
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let cols = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ])
    .split(inner);

    let section = |t: &str| {
        Line::from(Span::styled(
            t.to_string(),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
    };
    let key = |k: &str, d: &str| {
        Line::from(vec![
            Span::styled(format!("  {:<10}", k), Style::default().fg(WHITE)),
            Span::styled(d.to_string(), Style::default().fg(DIM)),
        ])
    };

    // ---- Left column: Navigation, Manage, Pricing -----------------------
    let mut left: Vec<Line> = Vec::new();
    left.push(section("Navigation"));
    left.push(key("↑ ↓ / j k", "Move selection"));
    left.push(key("Tab / ← →", "Switch panes"));
    left.push(key("Enter", "Open / edit"));
    left.push(Line::from(""));
    left.push(section("Manage"));
    left.push(key("n", "New collection or product"));
    left.push(key("e", "Edit or rename"));
    left.push(key("d", "Delete"));
    left.push(key("u", "Duplicate product"));
    left.push(key("m", "Move to another collection"));
    left.push(Line::from(""));
    left.push(section("Pricing"));
    left.push(key("c", "Compare on every platform"));
    left.push(key("p", "Change platform"));
    left.push(key("r", "Reverse-price from margin %"));
    left.push(key("s", "Cycle sort order"));
    left.push(key("%", "Sale scenario + fake sale calc"));
    left.push(key("w", "Wholesale view (direct sale)"));

    // ---- Right column: Data, Presets, General, footer -------------------
    let path = storage::data_file()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "(unknown)".into());

    let mut right: Vec<Line> = Vec::new();
    right.push(section("Search & Data"));
    right.push(key("/", "Search all products"));
    right.push(key("X", "Export spreadsheet (.xlsx)"));
    right.push(key("i", "Import spreadsheet (map columns)"));
    right.push(Line::from(""));
    right.push(section("Fee Presets"));
    right.push(key("f", "Manage custom presets"));
    right.push(Line::from(""));
    right.push(section("General"));
    right.push(key("g", "Settings (currency symbol)"));
    right.push(key("?", "This help"));
    right.push(key("Ctrl+C", "Quit"));
    right.push(Line::from(""));
    right.push(Line::from(Span::styled(
        format!("{} built-in presets", presets::builtin_presets().len()),
        Style::default().fg(DIM),
    )));
    right.push(Line::from(""));
    right.push(Line::from(Span::styled(
        "Data saves to:",
        Style::default().fg(DIM),
    )));
    right.push(Line::from(Span::styled(path, Style::default().fg(DIM))));

    f.render_widget(Paragraph::new(left).wrap(Wrap { trim: true }), cols[0]);
    f.render_widget(Paragraph::new(right).wrap(Wrap { trim: true }), cols[1]);
}
