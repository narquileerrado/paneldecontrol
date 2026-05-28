use crate::core::event::InputEvent;
use crate::widgets::{
    DataState, Widget, WidgetAction, WidgetConfig, WidgetContext, WidgetId, WidgetMsg,
    WorkerContext,
};
use anyhow::Result;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Paragraph, Wrap},
};
use std::time::Instant;

pub struct StaticWidget {
    id: WidgetId,
    text: String,
    started_at: Instant,
}

impl StaticWidget {
    pub async fn init(config: WidgetConfig, _ctx: WidgetContext) -> Result<Box<dyn Widget>> {
        let text = config
            .params
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(Box::new(Self {
            id: config.id,
            text,
            started_at: Instant::now(),
        }))
    }
}

impl Widget for StaticWidget {
    fn id(&self) -> &WidgetId {
        &self.id
    }
    fn kind(&self) -> &str {
        "static"
    }

    fn data_state(&self) -> DataState {
        DataState::Fresh {
            fetched_at: self.started_at,
        }
    }

    fn start_background(&mut self, _ctx: WorkerContext) {}
    fn update(&mut self, _msg: WidgetMsg) {}

    fn render(&self, area: Rect, buf: &mut Buffer) {
        ratatui::widgets::Widget::render(
            Paragraph::new(self.text.as_str()).wrap(Wrap { trim: true }),
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
