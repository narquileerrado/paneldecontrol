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

pub struct BtcWidget {
    id: WidgetId,
    ttl: Duration,
    lines: Vec<String>,
    last_fetch: Option<Instant>,
    worker: Option<AbortHandle>,
}

impl BtcWidget {
    pub async fn init(config: WidgetConfig, _ctx: WidgetContext) -> Result<Box<dyn Widget>> {
        let ttl_secs = config
            .params
            .get("ttl_secs")
            .and_then(|v| v.as_integer())
            .unwrap_or(60) as u64;
        Ok(Box::new(Self {
            id: config.id,
            ttl: Duration::from_secs(ttl_secs),
            lines: Vec::new(),
            last_fetch: None,
            worker: None,
        }))
    }
}

impl Widget for BtcWidget {
    fn id(&self) -> &WidgetId {
        &self.id
    }
    fn kind(&self) -> &str {
        "btc"
    }

    fn data_state(&self) -> DataState {
        match self.last_fetch {
            None => DataState::Loading,
            Some(t) => {
                if t.elapsed() > self.ttl + Duration::from_secs(60) {
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
                let msg = match fetch_btc(&client).await {
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
                tracing::warn!(id = self.id, error = e, "btc fetch error");
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
                Style::default().fg(Color::DarkGray),
            )
        } else {
            (self.lines.join("\n"), Style::default())
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
// Structs para la API de Binance
// ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinanceTicker {
    last_price: String,
    price_change_percent: String,
    high_price: String,
    low_price: String,
}

// ──────────────────────────────────────────────────────────────

async fn fetch_btc(client: &reqwest::Client) -> Result<Vec<String>> {
    let url = "https://api.binance.com/api/v3/ticker/24hr?symbol=BTCUSDT";
    let resp = client.get(url).send().await?;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {}", resp.status().as_u16()));
    }

    let ticker: BinanceTicker = resp.json().await?;
    Ok(format_lines(&ticker))
}

fn format_lines(t: &BinanceTicker) -> Vec<String> {
    let price: f64 = t.last_price.parse().unwrap_or(0.0);
    let change: f64 = t.price_change_percent.parse().unwrap_or(0.0);
    let high: f64 = t.high_price.parse().unwrap_or(0.0);
    let low: f64 = t.low_price.parse().unwrap_or(0.0);

    let arrow = if change >= 0.0 { "▲" } else { "▼" };
    let change_color_hint = if change >= 0.0 { "+" } else { "" };

    vec![
        "₿ Bitcoin / USD".to_string(),
        format!("  ${}", fmt_usd(price)),
        format!("  {}{}{:.2}%  24h", arrow, change_color_hint, change),
        format!("  H: ${}  L: ${}", fmt_usd(high), fmt_usd(low)),
    ]
}

fn fmt_usd(v: f64) -> String {
    let cents = ((v.fract()) * 100.0).round() as u32;
    let dollars = v as u64;
    let s = dollars.to_string();
    let with_sep = s
        .bytes()
        .rev()
        .collect::<Vec<_>>()
        .chunks(3)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(",")
        .bytes()
        .rev()
        .collect::<Vec<_>>();
    let int_part = String::from_utf8(with_sep).unwrap_or_default();
    format!("{}.{:02}", int_part, cents)
}

// ──────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "symbol": "BTCUSDT",
        "priceChange": "-123.45000000",
        "priceChangePercent": "-0.280",
        "weightedAvgPrice": "43374.28000000",
        "lastPrice": "43250.50000000",
        "lastQty": "0.00120000",
        "bidPrice": "43250.00000000",
        "askPrice": "43251.00000000",
        "openPrice": "43374.00000000",
        "highPrice": "43800.00000000",
        "lowPrice": "42900.00000000",
        "volume": "12345.67890000",
        "quoteVolume": "535678900.00000000",
        "openTime": 1705276800000,
        "closeTime": 1705363200000,
        "firstId": 123456789,
        "lastId": 123456999,
        "count": 210
    }"#;

    #[test]
    fn deserializa_correctamente() {
        let t: BinanceTicker = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(t.last_price, "43250.50000000");
        assert_eq!(t.price_change_percent, "-0.280");
        assert_eq!(t.high_price, "43800.00000000");
        assert_eq!(t.low_price, "42900.00000000");
    }

    #[test]
    fn formato_incluye_precio_y_cambio() {
        let t: BinanceTicker = serde_json::from_str(FIXTURE).unwrap();
        let lines = format_lines(&t);
        assert!(lines[1].contains("43,250"));
        assert!(lines[2].contains("▼"));
        assert!(lines[2].contains("-0.28%"));
    }

    #[test]
    fn fmt_usd_separadores() {
        assert_eq!(fmt_usd(43250.5), "43,250.50");
        assert_eq!(fmt_usd(1000.0), "1,000.00");
        assert_eq!(fmt_usd(999.99), "999.99");
        assert_eq!(fmt_usd(1000000.0), "1,000,000.00");
    }
}
