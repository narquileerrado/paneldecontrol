use crate::config::LayoutConfig;
use crate::core::event::{AppEvent, EventHandler, InputEvent};
use crate::core::layout::LayoutManager;
use crate::core::persist::AppState;
use crate::core::terminal::Tui;
use crate::theme::Theme;
use crate::widgets::{CoreMsg, DataState, Widget, WidgetAction, WorkerContext};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Borders},
    Frame,
};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct App {
    running: bool,
    theme: Theme,
    layout: LayoutManager,
    widgets: HashMap<String, Box<dyn Widget>>,
    core_tx: mpsc::Sender<CoreMsg>,
    core_rx: Option<mpsc::Receiver<CoreMsg>>,
    last_area: Rect,
}

impl App {
    pub fn new(
        layout_config: LayoutConfig,
        widgets: HashMap<String, Box<dyn Widget>>,
        theme: Theme,
    ) -> Self {
        let (core_tx, core_rx) = mpsc::channel(256);
        Self {
            running: true,
            theme,
            layout: LayoutManager::new(layout_config),
            widgets,
            core_tx,
            core_rx: Some(core_rx),
            last_area: Rect::default(),
        }
    }

    pub async fn run(&mut self, mut terminal: Tui) -> Result<()> {
        let size = terminal.size()?;
        self.last_area = Rect::new(0, 0, size.width, size.height);
        self.layout.recalculate(self.last_area);

        for (id, widget) in &mut self.widgets {
            widget.start_background(WorkerContext {
                widget_id: id.clone(),
                tx: self.core_tx.clone(),
            });
        }

        let core_rx = self.core_rx.take().expect("run() llamado dos veces");
        let mut events = EventHandler::new(Duration::from_millis(250), core_rx);

        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            match events.next().await? {
                AppEvent::Input(ev) => self.handle_input(ev),
                AppEvent::Core(CoreMsg { widget_id, msg }) => {
                    if let Some(w) = self.widgets.get_mut(&widget_id) {
                        w.update(msg);
                    }
                }
            }
        }

        self.save_state();
        for widget in self.widgets.values_mut() {
            widget.stop();
        }
        Ok(())
    }

    fn handle_input(&mut self, event: InputEvent) {
        match event {
            InputEvent::Key(key) => {
                // Shortcuts globales que siempre tienen prioridad.
                match key.code {
                    KeyCode::Char('q') => {
                        self.running = false;
                        return;
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.running = false;
                        return;
                    }
                    KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.save_state();
                        tracing::info!("layout guardado");
                        return;
                    }
                    KeyCode::Tab => {
                        self.layout.focus_next();
                        return;
                    }
                    KeyCode::BackTab => {
                        self.layout.focus_prev();
                        return;
                    }
                    _ => {}
                }

                // Delegar al widget enfocado.
                let focused_id = {
                    let idx = self.layout.focused_id();
                    self.layout.slots().get(idx).map(|s| s.widget_id.clone())
                };
                let consumed = if let Some(id) = focused_id {
                    if let Some(w) = self.widgets.get_mut(&id) {
                        matches!(w.handle_input(InputEvent::Key(key)), WidgetAction::Consumed)
                    } else {
                        false
                    }
                } else {
                    false
                };
                if consumed {
                    return;
                }

                // Resize interactivo (solo si el widget no consumió el evento).
                match key.code {
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        self.layout.resize_focused_width(5);
                        self.layout.recalculate(self.last_area);
                    }
                    KeyCode::Char('-') => {
                        self.layout.resize_focused_width(-5);
                        self.layout.recalculate(self.last_area);
                    }
                    KeyCode::Char('{') => {
                        self.layout.resize_focused_height(-5);
                        self.layout.recalculate(self.last_area);
                    }
                    KeyCode::Char('}') => {
                        self.layout.resize_focused_height(5);
                        self.layout.recalculate(self.last_area);
                    }
                    _ => {}
                }
            }
            InputEvent::Mouse(m) => tracing::debug!(?m, "mouse"),
            InputEvent::Resize(w, h) => {
                self.last_area = Rect::new(0, 0, w, h);
                self.layout.recalculate(self.last_area);
            }
            InputEvent::Tick => {}
        }
    }

    fn render(&self, frame: &mut Frame) {
        let buf = frame.buffer_mut();

        for (i, slot) in self.layout.slots().iter().enumerate() {
            let focused = i == self.layout.focused_id();

            if slot.area.width < 10 || slot.area.height < 4 {
                ratatui::widgets::Widget::render(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_type(self.theme.border_style)
                        .border_style(Style::default().fg(self.theme.inactive)),
                    slot.area,
                    buf,
                );
                continue;
            }

            let color = if focused {
                self.theme.accent_focused
            } else {
                self.theme.accent_for(&slot.widget_id)
            };

            let title = match self.widgets.get(&slot.widget_id) {
                Some(w) => {
                    let suffix = match w.data_state() {
                        DataState::Loading => " [~]",
                        DataState::Fresh { .. } => "",
                        DataState::Stale { .. } => " [!]",
                        DataState::Error(_) => " [E]",
                    };
                    format!(" {}{} ", slot.widget_id, suffix)
                }
                None => format!(" {} ", slot.widget_id),
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(self.theme.border_style)
                .border_style(Style::default().fg(color))
                .title(title)
                .title_style(Style::default().fg(color));

            let inner = block.inner(slot.area);
            ratatui::widgets::Widget::render(block, slot.area, buf);

            if let Some(w) = self.widgets.get(&slot.widget_id) {
                w.render(inner, buf);
            }
        }
    }

    fn save_state(&self) {
        AppState::from_layout(self.layout.export_config()).save();
    }
}
