mod config;
mod core;
mod dirs;
mod theme;
mod widgets;

use anyhow::Result;
use std::sync::Mutex;

#[tokio::main]
async fn main() -> Result<()> {
    init_logging()?;

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        core::terminal::restore_terminal().ok();
        original_hook(info);
    }));

    let mut config = config::Config::load_or_default();

    // Aplicar layout guardado (resize interactivo persiste entre sesiones).
    let state = core::persist::AppState::load();
    if let Some(ref snap) = state.layout {
        snap.apply_to(&mut config.layout);
    }

    let theme = theme::Theme::from_config(&config.theme);
    let registry = widgets::WidgetRegistry::new();
    let widget_map = registry.build_all(&config).await;

    let terminal = core::terminal::init_terminal()?;
    let result = core::app::App::new(config.layout, widget_map, theme)
        .run(terminal)
        .await;
    core::terminal::restore_terminal()?;
    result
}

fn init_logging() -> Result<()> {
    let log_file = std::fs::File::create("/tmp/paneldecontrol.log")?;
    tracing_subscriber::fmt()
        .with_writer(Mutex::new(log_file))
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .init();
    Ok(())
}
