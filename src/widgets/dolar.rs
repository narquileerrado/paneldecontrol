use crate::core::event::InputEvent;
use crate::widgets::{
    CoreMsg, DataState, Widget, WidgetAction, WidgetConfig, WidgetContext, WidgetId, WidgetMsg,
    WorkerContext,
};
use anyhow::Result;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Paragraph, Wrap},
};
use serde::Deserialize;
use std::time::{Duration, Instant};
use tokio::task::AbortHandle;

pub struct DolarWidget {
    id: WidgetId,
    ttl: Duration,
    lines: Vec<String>,
    last_fetch: Option<Instant>,
    worker: Option<AbortHandle>,
}

impl DolarWidget {
    pub async fn init(config: WidgetConfig, _ctx: WidgetContext) -> Result<Box<dyn Widget>> {
        let ttl_secs = config
            .params
            .get("ttl_secs")
            .and_then(|v| v.as_integer())
            .unwrap_or(300) as u64;
        Ok(Box::new(Self {
            id: config.id,
            ttl: Duration::from_secs(ttl_secs),
            lines: Vec::new(),
            last_fetch: None,
            worker: None,
        }))
    }
}

impl Widget for DolarWidget {
    fn id(&self) -> &WidgetId {
        &self.id
    }
    fn kind(&self) -> &str {
        "dolar"
    }

    fn data_state(&self) -> DataState {
        match self.last_fetch {
            None => DataState::Loading,
            Some(t) => {
                if t.elapsed() > self.ttl + Duration::from_secs(120) {
                    DataState::Stale { fetched_at: t }
                } else {
                    DataState::Fresh { fetched_at: t }
                }
            }
        }
    }

    fn start_background(&mut self, ctx: WorkerContext) {
        let ttl = self.ttl;
        let tx = ctx.tx;
        let widget_id = ctx.widget_id;

        let handle = tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .user_agent("Mozilla/5.0 (compatible; paneldecontrol/0.1)")
                .build()
                .expect("HTTP client");

            let mut attempts: u32 = 0;
            loop {
                let msg = match fetch_dolar(&client).await {
                    Ok(lines) => {
                        attempts = 0;
                        WidgetMsg::Lines(lines)
                    }
                    Err(e) => {
                        attempts = attempts.saturating_add(1);
                        WidgetMsg::Error(e.to_string())
                    }
                };
                let _ = tx
                    .send(CoreMsg {
                        widget_id: widget_id.clone(),
                        msg,
                    })
                    .await;

                let wait = if attempts == 0 {
                    ttl
                } else {
                    Duration::from_secs(30u64.saturating_mul(1 << attempts.min(5)))
                };
                tokio::time::sleep(wait).await;
            }
        });
        self.worker = Some(handle.abort_handle());
    }

    fn stop(&mut self) {
        if let Some(h) = self.worker.take() {
            h.abort();
        }
    }

    fn update(&mut self, msg: WidgetMsg) {
        match msg {
            WidgetMsg::Lines(lines) => {
                self.lines = lines;
                self.last_fetch = Some(Instant::now());
            }
            WidgetMsg::Error(e) => {
                tracing::warn!(id = self.id, error = e, "dolar fetch error");
                if self.lines.is_empty() {
                    self.lines = vec![format!("Error: {}", e)];
                }
            }
            _ => {}
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let (text, style) = if self.lines.is_empty() {
            (
                "Cargando...".to_string(),
                Style::default().fg(Color::Rgb(0x80, 0x58, 0x00)),
            )
        } else {
            (
                self.lines.join("\n"),
                Style::default().fg(Color::Rgb(0xff, 0xb0, 0x00)),
            )
        };
        ratatui::widgets::Widget::render(
            Paragraph::new(text).style(style).wrap(Wrap { trim: false }),
            area,
            buf,
        );
    }

    fn handle_input(&mut self, _ev: InputEvent) -> WidgetAction {
        WidgetAction::None
    }

    fn serialize_state(&self) -> toml::Value {
        toml::Value::Table(toml::map::Map::new())
    }
}

// ──────────────────────────────────────────────────────────────
// Structs de la API bluelytics
// ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct BluelyticsResp {
    oficial: Cotizacion,
    blue: Cotizacion,
}

#[derive(Deserialize)]
struct Cotizacion {
    value_buy: f64,
    value_sell: f64,
}

// ──────────────────────────────────────────────────────────────

async fn fetch_dolar(client: &reqwest::Client) -> Result<Vec<String>> {
    let url = "https://api.bluelytics.com.ar/v2/latest";
    let resp = client.get(url).send().await?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {}", resp.status().as_u16()));
    }

    let data: BluelyticsResp = resp.json().await?;
    Ok(format_lines(&data))
}

fn format_lines(data: &BluelyticsResp) -> Vec<String> {
    let spread_abs = data.blue.value_sell - data.oficial.value_sell;
    let spread_pct = spread_abs / data.oficial.value_sell * 100.0;

    vec![
        "$ Dólar AR".to_string(),
        format!("         {:>8}  {:>8}", "Compra", "Venta"),
        format!(
            "Oficial  {:>8}  {:>8}",
            fmt_ars(data.oficial.value_buy),
            fmt_ars(data.oficial.value_sell)
        ),
        format!(
            "Blue     {:>8}  {:>8}",
            fmt_ars(data.blue.value_buy),
            fmt_ars(data.blue.value_sell)
        ),
        format!("Brecha   {:>+.1}%", spread_pct),
    ]
}

fn fmt_ars(v: f64) -> String {
    format!("${:.0}", v)
}

// ──────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "oficial": { "value_buy": 1035.5, "value_sell": 1075.5, "date": "2024-01-15 09:00:00" },
        "blue":    { "value_buy": 1150.0, "value_sell": 1200.0, "date": "2024-01-15 10:30:00" },
        "oficial_euro": { "value_buy": 1100.0, "value_sell": 1150.0, "date": "2024-01-15 09:00:00" },
        "blue_euro":    { "value_buy": 1250.0, "value_sell": 1300.0, "date": "2024-01-15 10:30:00" },
        "last_update": "2024-01-15T10:30:00.000Z"
    }"#;

    #[test]
    fn deserializa_correctamente() {
        let data: BluelyticsResp = serde_json::from_str(FIXTURE).unwrap();
        assert!((data.oficial.value_buy - 1035.5).abs() < 0.01);
        assert!((data.oficial.value_sell - 1075.5).abs() < 0.01);
        assert!((data.blue.value_buy - 1150.0).abs() < 0.01);
        assert!((data.blue.value_sell - 1200.0).abs() < 0.01);
    }

    #[test]
    fn formato_incluye_brecha() {
        let data: BluelyticsResp = serde_json::from_str(FIXTURE).unwrap();
        let lines = format_lines(&data);
        let brecha_line = lines.iter().find(|l| l.contains("Brecha")).unwrap();
        // 1200 / 1075.5 ≈ 11.6%
        assert!(brecha_line.contains('+'), "debe mostrar brecha positiva");
    }

    #[test]
    fn formato_contiene_oficial_y_blue() {
        let data: BluelyticsResp = serde_json::from_str(FIXTURE).unwrap();
        let lines = format_lines(&data);
        assert!(lines.iter().any(|l| l.contains("Oficial")));
        assert!(lines.iter().any(|l| l.contains("Blue")));
    }
}
