//! VORA-Margin — a friendly TUI price calculator for clothing brands.
//!
//! A spreadsheet replacement: organize products into collections, pick the
//! platform you sell on (Stripe, Depop, Etsy, Vinted...), and instantly see
//! your margin after every fee. Everything is stored locally.

mod app;
mod calc;
mod model;
mod presets;
mod storage;
mod textfield;
mod ui;
mod util;
mod xlsx_export;
mod xlsx_import;

use anyhow::Result;
use app::App;
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;

fn main() -> Result<()> {
    let mut app = App::new()?;
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|f| ui::render(f, app))?;
        // Blocking read — no busy loop, no wasted CPU.
        if let Event::Key(key) = event::read()? {
            // On Windows crossterm emits press *and* release; only act on press.
            if key.kind == KeyEventKind::Press {
                app.handle_key(key);
            }
        }
    }
    Ok(())
}
