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

pub struct WeatherWidget {
    id: WidgetId,
    location: String,
    api_key: String,
    ttl: Duration,
    lines: Vec<String>,
    last_fetch: Option<Instant>,
    worker: Option<AbortHandle>,
}

impl WeatherWidget {
    pub async fn init(config: WidgetConfig, _ctx: WidgetContext) -> Result<Box<dyn Widget>> {
        let location = config
            .params
            .get("location")
            .and_then(|v| v.as_str())
            .unwrap_or("Buenos Aires,AR")
            .to_string();
        let api_key = config
            .params
            .get("api_key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let ttl_secs = config
            .params
            .get("ttl_secs")
            .and_then(|v| v.as_integer())
            .unwrap_or(600) as u64;

        Ok(Box::new(Self {
            id: config.id,
            location,
            api_key,
            ttl: Duration::from_secs(ttl_secs),
            lines: Vec::new(),
            last_fetch: None,
            worker: None,
        }))
    }
}

impl Widget for WeatherWidget {
    fn id(&self) -> &WidgetId {
        &self.id
    }
    fn kind(&self) -> &str {
        "weather"
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
        let location = self.location.clone();
        let api_key = self.api_key.clone();
        let ttl = self.ttl;
        let tx = ctx.tx;
        let widget_id = ctx.widget_id;

        let handle = tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("HTTP client");

            let mut attempts: u32 = 0;
            loop {
                let msg = match fetch_all(&client, &location, &api_key).await {
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
                tracing::warn!(id = self.id, error = e, "weather fetch error");
                if self.lines.is_empty() {
                    self.lines = vec![format!("Error: {}", e)];
                }
            }
            WidgetMsg::Entries(_) | WidgetMsg::Sistema(_) => {}
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let text = if self.lines.is_empty() {
            "Cargando...".to_string()
        } else {
            self.lines.join("\n")
        };
        let style = if self.lines.is_empty() {
            Style::default().fg(Color::Rgb(0x80, 0x58, 0x00))
        } else {
            Style::default().fg(Color::Rgb(0xff, 0xb0, 0x00))
        };
        ratatui::widgets::Widget::render(
            Paragraph::new(text).style(style).wrap(Wrap { trim: true }),
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
// Structs para deserializar las respuestas de OpenWeather
// ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct WeatherResponse {
    weather: Vec<WeatherCondition>,
    main: MainData,
    wind: WindData,
    name: String,
}

#[derive(Deserialize)]
struct ForecastResponse {
    list: Vec<ForecastItem>,
}

#[derive(Deserialize)]
struct ForecastItem {
    dt: i64,
    main: MainData,
    weather: Vec<WeatherCondition>,
}

#[derive(Deserialize)]
struct WeatherCondition {
    description: String,
}

#[derive(Deserialize)]
struct MainData {
    temp: f32,
    feels_like: f32,
    humidity: u32,
}

#[derive(Deserialize)]
struct WindData {
    speed: f32,
}

// ──────────────────────────────────────────────────────────────

async fn fetch_all(client: &reqwest::Client, location: &str, api_key: &str) -> Result<Vec<String>> {
    if api_key.is_empty() {
        return Ok(vec!["api_key no configurada en config.toml".to_string()]);
    }

    let mut lines = fetch_current(client, location, api_key).await?;

    if let Ok(forecast) = fetch_forecast(client, location, api_key).await {
        lines.push(String::new());
        lines.push("── Próximas 24 hs ──────────────".to_string());
        lines.extend(forecast);
    }

    Ok(lines)
}

async fn fetch_current(
    client: &reqwest::Client,
    location: &str,
    api_key: &str,
) -> Result<Vec<String>> {
    let url = format!(
        "https://api.openweathermap.org/data/2.5/weather\
         ?q={location}&appid={api_key}&units=metric&lang=es"
    );

    let response = client.get(&url).send().await?;
    let status = response.status();
    let text = response.text().await?;

    if !status.is_success() {
        let api_msg = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("message")?.as_str().map(String::from))
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        return Err(anyhow::anyhow!("{}", api_msg));
    }

    let resp: WeatherResponse = serde_json::from_str(&text)?;

    let desc = resp
        .weather
        .first()
        .map(|w| {
            let mut d = w.description.clone();
            if let Some(c) = d.get_mut(0..1) {
                c.make_ascii_uppercase();
            }
            d
        })
        .unwrap_or_default();

    Ok(vec![
        resp.name,
        desc,
        format!(
            "Temp:    {:.1}°C  (sens. {:.1}°C)",
            resp.main.temp, resp.main.feels_like
        ),
        format!(
            "Humedad: {}%   Viento: {:.1} m/s",
            resp.main.humidity, resp.wind.speed
        ),
    ])
}

async fn fetch_forecast(
    client: &reqwest::Client,
    location: &str,
    api_key: &str,
) -> Result<Vec<String>> {
    let url = format!(
        "https://api.openweathermap.org/data/2.5/forecast\
         ?q={location}&appid={api_key}&units=metric&lang=es&cnt=8"
    );

    let response = client.get(&url).send().await?;
    let status = response.status();
    let text = response.text().await?;

    if !status.is_success() {
        return Err(anyhow::anyhow!("HTTP {}", status.as_u16()));
    }

    let forecast: ForecastResponse = serde_json::from_str(&text)?;

    let lines = forecast
        .list
        .iter()
        .map(|item| {
            let time = chrono::DateTime::from_timestamp(item.dt, 0)
                .map(|utc: chrono::DateTime<chrono::Utc>| {
                    utc.with_timezone(&chrono::Local)
                        .format("%H:%M")
                        .to_string()
                })
                .unwrap_or_else(|| "--:--".to_string());

            let desc = item
                .weather
                .first()
                .map(|w| w.description.as_str())
                .unwrap_or("");

            format!("  {}  {:5.1}°C  {}", time, item.main.temp, desc)
        })
        .collect();

    Ok(lines)
}

// ──────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CURRENT: &str = r#"{
        "weather": [{"id":800,"main":"Clear","description":"cielo despejado","icon":"01d"}],
        "main": {"temp":22.5,"feels_like":20.8,"temp_min":20.0,"temp_max":24.0,
                 "pressure":1015,"humidity":55},
        "wind": {"speed":3.6,"deg":160},
        "name": "Buenos Aires"
    }"#;

    const SAMPLE_FORECAST: &str = r#"{
        "list": [
            {
                "dt": 1716800400,
                "main": {"temp":23.1,"feels_like":21.5,"temp_min":22.0,"temp_max":24.0,
                         "pressure":1012,"humidity":60},
                "weather": [{"id":801,"main":"Clouds","description":"algo de nubes","icon":"02d"}],
                "wind": {"speed":4.2,"deg":180}
            },
            {
                "dt": 1716811200,
                "main": {"temp":21.0,"feels_like":19.8,"temp_min":20.0,"temp_max":22.0,
                         "pressure":1013,"humidity":65},
                "weather": [{"id":800,"main":"Clear","description":"cielo despejado","icon":"01n"}],
                "wind": {"speed":3.1,"deg":170}
            }
        ]
    }"#;

    #[test]
    fn deserializa_current_correctamente() {
        let resp: WeatherResponse = serde_json::from_str(SAMPLE_CURRENT).unwrap();
        assert_eq!(resp.name, "Buenos Aires");
        assert!((resp.main.temp - 22.5).abs() < 0.01);
        assert_eq!(resp.main.humidity, 55);
        assert_eq!(resp.weather[0].description, "cielo despejado");
        assert!((resp.wind.speed - 3.6).abs() < 0.01);
    }

    #[test]
    fn deserializa_forecast_correctamente() {
        let f: ForecastResponse = serde_json::from_str(SAMPLE_FORECAST).unwrap();
        assert_eq!(f.list.len(), 2);
        assert!((f.list[0].main.temp - 23.1).abs() < 0.01);
        assert_eq!(f.list[1].weather[0].description, "cielo despejado");
    }

    #[test]
    fn formato_forecast_incluye_temp_y_desc() {
        let f: ForecastResponse = serde_json::from_str(SAMPLE_FORECAST).unwrap();
        let line = format!(
            "  {:5.1}°C  {}",
            f.list[0].main.temp, f.list[0].weather[0].description
        );
        assert!(line.contains("23.1"));
        assert!(line.contains("algo de nubes"));
    }

    #[test]
    fn formato_current_incluye_datos_clave() {
        let resp: WeatherResponse = serde_json::from_str(SAMPLE_CURRENT).unwrap();
        let lines = vec![
            resp.name.clone(),
            resp.weather[0].description.clone(),
            format!(
                "Temp:    {:.1}°C  (sens. {:.1}°C)",
                resp.main.temp, resp.main.feels_like
            ),
            format!(
                "Humedad: {}%   Viento: {:.1} m/s",
                resp.main.humidity, resp.wind.speed
            ),
        ];
        assert!(lines[2].contains("22.5"));
        assert!(lines[3].contains("55%"));
        assert!(lines[3].contains("3.6"));
    }
}
