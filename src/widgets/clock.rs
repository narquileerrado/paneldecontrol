use crate::core::event::InputEvent;
use crate::widgets::{
    DataState, Widget, WidgetAction, WidgetConfig, WidgetContext, WidgetId, WidgetMsg,
    WorkerContext,
};
use anyhow::Result;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Color, Style},
    widgets::Paragraph,
};
use std::time::Instant;

pub struct ClockWidget {
    id: WidgetId,
    started_at: Instant,
}

impl ClockWidget {
    pub async fn init(config: WidgetConfig, _ctx: WidgetContext) -> Result<Box<dyn Widget>> {
        Ok(Box::new(Self {
            id: config.id,
            started_at: Instant::now(),
        }))
    }
}

impl Widget for ClockWidget {
    fn id(&self) -> &WidgetId {
        &self.id
    }
    fn kind(&self) -> &str {
        "clock"
    }

    fn data_state(&self) -> DataState {
        DataState::Fresh {
            fetched_at: self.started_at,
        }
    }

    fn start_background(&mut self, _ctx: WorkerContext) {}
    fn update(&mut self, _msg: WidgetMsg) {}

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }

        let now = chrono::Local::now();
        let time = now.format("%H:%M:%S").to_string();
        let date = date_es(&now);

        let v_offset = area.height.saturating_sub(2) / 2;

        let time_area = Rect::new(area.x, area.y + v_offset, area.width, 1);
        ratatui::widgets::Widget::render(
            Paragraph::new(time).alignment(Alignment::Center),
            time_area,
            buf,
        );

        if area.height >= 2 {
            let date_area = Rect::new(area.x, area.y + v_offset + 1, area.width, 1);
            ratatui::widgets::Widget::render(
                Paragraph::new(date)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::DarkGray)),
                date_area,
                buf,
            );
        }
    }

    fn handle_input(&mut self, _ev: InputEvent) -> WidgetAction {
        WidgetAction::None
    }

    fn serialize_state(&self) -> toml::Value {
        toml::Value::Table(toml::map::Map::new())
    }
}

fn date_es(dt: &chrono::DateTime<chrono::Local>) -> String {
    use chrono::{Datelike, Weekday};
    let dia = match dt.weekday() {
        Weekday::Mon => "lunes",
        Weekday::Tue => "martes",
        Weekday::Wed => "miércoles",
        Weekday::Thu => "jueves",
        Weekday::Fri => "viernes",
        Weekday::Sat => "sábado",
        Weekday::Sun => "domingo",
    };
    let mes = match dt.month() {
        1 => "enero",
        2 => "febrero",
        3 => "marzo",
        4 => "abril",
        5 => "mayo",
        6 => "junio",
        7 => "julio",
        8 => "agosto",
        9 => "septiembre",
        10 => "octubre",
        11 => "noviembre",
        12 => "diciembre",
        _ => "",
    };
    format!("{}, {} de {} de {}", dia, dt.day(), mes, dt.year())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    #[test]
    fn date_es_miercoles_mayo() {
        // 2026-05-27 es miércoles
        let dt = Local.with_ymd_and_hms(2026, 5, 27, 12, 0, 0).unwrap();
        assert_eq!(date_es(&dt), "miércoles, 27 de mayo de 2026");
    }

    #[test]
    fn date_es_todos_los_meses() {
        let meses = [
            "enero",
            "febrero",
            "marzo",
            "abril",
            "mayo",
            "junio",
            "julio",
            "agosto",
            "septiembre",
            "octubre",
            "noviembre",
            "diciembre",
        ];
        for (i, nombre) in meses.iter().enumerate() {
            let dt = Local
                .with_ymd_and_hms(2026, (i + 1) as u32, 1, 0, 0, 0)
                .unwrap();
            assert!(
                date_es(&dt).contains(nombre),
                "mes {} no encontrado",
                nombre
            );
        }
    }
}
