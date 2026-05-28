#![allow(dead_code)]

pub mod btc;
pub mod clock;
pub mod dolar;
pub mod rss;
pub mod sistema;
pub mod static_text;
pub mod weather;

use crate::config::Config;
use crate::core::event::InputEvent;
use anyhow::Result;
use ratatui::{buffer::Buffer, layout::Rect};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

pub type WidgetId = String;

// Estado del dato que el renderer muestra en el header del slot.
pub enum DataState {
    Loading,
    Fresh { fetched_at: Instant },
    Stale { fetched_at: Instant },
    Error(String),
}

#[derive(Clone)]
pub struct WidgetConfig {
    pub id: WidgetId,
    pub kind: String,
    pub params: HashMap<String, toml::Value>,
}

// Datos en bruto del widget sistema (CPU, RAM, discos).
pub struct SistemaMetrics {
    pub cpu_pct: f32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub disks: Vec<(String, u64, u64)>, // (label, used, total)
}

// Mensaje que el worker envía al widget vía canal.
pub enum WidgetMsg {
    Lines(Vec<String>),
    Entries(Vec<(String, String)>), // (título, resumen/cuerpo)
    Error(String),
    Sistema(SistemaMetrics),
}

// Mensaje genérico que el worker envía al core.
pub struct CoreMsg {
    pub widget_id: String,
    pub msg: WidgetMsg,
}

// Contexto que el core le pasa al widget al iniciar su background task.
pub struct WorkerContext {
    pub widget_id: String,
    pub tx: tokio::sync::mpsc::Sender<CoreMsg>,
}

// Sprint 4: WidgetContext sigue vacío; Sprint 5 agregará tx_core y theme (via Arc<>).
#[derive(Clone, Copy)]
pub struct WidgetContext;

pub enum WidgetAction {
    None,
    ForceRefresh,
    Consumed,
}

// ──────────────────────────────────────────────────────────────
// Trait principal.
// ──────────────────────────────────────────────────────────────
pub trait Widget: Send {
    fn id(&self) -> &WidgetId;
    fn kind(&self) -> &str;
    fn data_state(&self) -> DataState;
    fn min_size(&self) -> (u16, u16) {
        (10, 4)
    }

    fn start_background(&mut self, ctx: WorkerContext);
    fn stop(&mut self) {} // override para abortar el worker
    fn update(&mut self, msg: WidgetMsg);
    fn render(&self, area: Rect, buf: &mut Buffer);
    fn handle_input(&mut self, ev: InputEvent) -> WidgetAction;
    fn serialize_state(&self) -> toml::Value;
}

// ──────────────────────────────────────────────────────────────
// Factory y Registry
// ──────────────────────────────────────────────────────────────
pub type WidgetFactory = fn(
    WidgetConfig,
    WidgetContext,
) -> Pin<Box<dyn Future<Output = Result<Box<dyn Widget>>> + Send>>;

pub struct WidgetRegistry {
    factories: HashMap<String, WidgetFactory>,
}

impl WidgetRegistry {
    pub fn new() -> Self {
        let mut r = Self {
            factories: HashMap::new(),
        };
        r.register("clock", |c, x| Box::pin(clock::ClockWidget::init(c, x)));
        r.register("static", |c, x| {
            Box::pin(static_text::StaticWidget::init(c, x))
        });
        r.register("weather", |c, x| {
            Box::pin(weather::WeatherWidget::init(c, x))
        });
        r.register("rss", |c, x| Box::pin(rss::RssWidget::init(c, x)));
        r.register("sistema", |c, x| {
            Box::pin(sistema::SistemaWidget::init(c, x))
        });
        r.register("dolar", |c, x| Box::pin(dolar::DolarWidget::init(c, x)));
        r.register("btc", |c, x| Box::pin(btc::BtcWidget::init(c, x)));
        r
    }

    pub fn register(&mut self, kind: &str, factory: WidgetFactory) {
        self.factories.insert(kind.to_string(), factory);
    }

    pub async fn build(&self, config: WidgetConfig, ctx: WidgetContext) -> Result<Box<dyn Widget>> {
        let factory = self
            .factories
            .get(&config.kind)
            .ok_or_else(|| anyhow::anyhow!("widget kind '{}' no registrado", config.kind))?;
        factory(config, ctx).await
    }

    pub async fn build_all(&self, config: &Config) -> HashMap<String, Box<dyn Widget>> {
        let mut map: HashMap<String, Box<dyn Widget>> = HashMap::new();
        for row in &config.layout.rows {
            for slot in &row.slots {
                if map.contains_key(&slot.widget) {
                    continue;
                }
                let Some(def) = config.widgets.get(&slot.widget) else {
                    tracing::warn!(id = slot.widget, "sin definición en [widgets]");
                    continue;
                };
                let wcfg = WidgetConfig {
                    id: slot.widget.clone(),
                    kind: def.kind.clone(),
                    params: def.params.clone(),
                };
                match self.build(wcfg, WidgetContext).await {
                    Ok(w) => {
                        map.insert(slot.widget.clone(), w);
                    }
                    Err(e) => tracing::error!(id = slot.widget, %e, "fallo al construir widget"),
                }
            }
        }
        map
    }
}
