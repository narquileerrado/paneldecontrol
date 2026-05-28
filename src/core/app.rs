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
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

const AMBER: Color = Color::Rgb(0xff, 0xb0, 0x00);
const AMBER_DIM: Color = Color::Rgb(0x80, 0x58, 0x00);
const GREEN: Color = Color::Rgb(0x4a, 0xf6, 0x26);

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠇"];

const GLITCH_NOISE: &[char] = &['▒', '░', '╫', '▓', '▀', '■', '╬', '▄', '▌', '╪'];

pub struct App {
    running: bool,
    theme: Theme,
    layout: LayoutManager,
    widgets: HashMap<String, Box<dyn Widget>>,
    core_tx: mpsc::Sender<CoreMsg>,
    core_rx: Option<mpsc::Receiver<CoreMsg>>,
    last_area: Rect,
    show_help: bool,
    tick_count: u64,
    glitch: Option<(usize, u8)>, // (slot_idx, frames_remaining)
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
            show_help: false,
            tick_count: 0,
            glitch: None,
        }
    }

    pub async fn run(&mut self, mut terminal: Tui) -> Result<()> {
        let size = terminal.size()?;
        self.last_area = Rect::new(0, 0, size.width, size.height);
        self.layout.recalculate(Self::content_area(self.last_area));
        self.layout.focus_widget_id("noticias");

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
                // Cuando el popup de ayuda está abierto, bloquear todo excepto cierre.
                if self.show_help {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('h') | KeyCode::Char('H') => {
                            self.show_help = false;
                        }
                        _ => {}
                    }
                    return;
                }

                // Shortcuts globales prioritarios.
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
                    KeyCode::Char('h') | KeyCode::Char('H') => {
                        self.show_help = true;
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

                // Resize interactivo.
                match key.code {
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        self.layout.resize_focused_width(5);
                        self.layout.recalculate(Self::content_area(self.last_area));
                    }
                    KeyCode::Char('-') => {
                        self.layout.resize_focused_width(-5);
                        self.layout.recalculate(Self::content_area(self.last_area));
                    }
                    KeyCode::Char('{') => {
                        self.layout.resize_focused_height(-5);
                        self.layout.recalculate(Self::content_area(self.last_area));
                    }
                    KeyCode::Char('}') => {
                        self.layout.resize_focused_height(5);
                        self.layout.recalculate(Self::content_area(self.last_area));
                    }
                    _ => {}
                }
            }
            InputEvent::Mouse(m) => tracing::debug!(?m, "mouse"),
            InputEvent::Resize(w, h) => {
                self.last_area = Rect::new(0, 0, w, h);
                self.layout.recalculate(Self::content_area(self.last_area));
            }
            InputEvent::Tick => {
                self.tick_count = self.tick_count.wrapping_add(1);

                // Decrementar glitch activo.
                self.glitch = match self.glitch {
                    Some((idx, f)) if f > 1 => Some((idx, f - 1)),
                    _ => None,
                };

                // Intentar disparar glitch nuevo (~5% de prob. cada ~1 s).
                if self.glitch.is_none() && self.tick_count.is_multiple_of(4) {
                    let r = pseudo_rand(self.tick_count);
                    if r.is_multiple_of(20) {
                        let n = self.layout.slots().len() as u64;
                        if n > 0 {
                            let idx = (r / 20 % n) as usize;
                            self.glitch = Some((idx, 2)); // ~500 ms
                        }
                    }
                }
            }
        }
    }

    fn render(&self, frame: &mut Frame) {
        let buf = frame.buffer_mut();
        let content = Self::content_area(self.last_area);
        let statusbar = Self::statusbar_area(self.last_area);

        self.render_widgets(content, buf);
        self.render_statusbar(statusbar, buf);

        if self.show_help {
            self.render_help(self.last_area, buf);
        }
    }

    fn render_widgets(&self, _content_area: Rect, buf: &mut ratatui::buffer::Buffer) {
        for (i, slot) in self.layout.slots().iter().enumerate() {
            let focused = i == self.layout.focused_id();
            let glitching = self.glitch.is_some_and(|(idx, _)| idx == i);

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
            } else if glitching {
                GREEN
            } else {
                self.theme.accent_for(&slot.widget_id)
            };

            let suffix = match self.widgets.get(&slot.widget_id) {
                Some(w) => match w.data_state() {
                    DataState::Loading => {
                        format!(" {}", SPINNER[self.tick_count as usize % SPINNER.len()])
                    }
                    DataState::Fresh { .. } => String::new(),
                    DataState::Stale { .. } => " [!]".to_string(),
                    DataState::Error(_) => " [E]".to_string(),
                },
                None => String::new(),
            };

            let raw_title = format!(" {}{} ", slot.widget_id, suffix);
            let title = if glitching {
                glitch_title(&raw_title, pseudo_rand(self.tick_count))
            } else {
                raw_title
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

    fn render_statusbar(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let left = " \u{2593}\u{2592}\u{2591} PANELDECONTROL // BA_AR_01 // ONLINE \u{2591}\u{2592}\u{2593}";
        let right = "[TAB:FOCO]  [H:AYUDA]  [Q:SALIR] ";

        let left_w = left.chars().count();
        let right_w = right.chars().count();
        let total_w = area.width as usize;
        let pad = total_w.saturating_sub(left_w + right_w);

        let line = Line::from(vec![
            Span::styled(left.to_string(), Style::default().fg(AMBER_DIM)),
            Span::raw(" ".repeat(pad)),
            Span::styled(right.to_string(), Style::default().fg(AMBER_DIM)),
        ]);

        ratatui::widgets::Widget::render(
            Paragraph::new(line).style(Style::default().bg(Color::Rgb(0x0f, 0x0c, 0x00))),
            area,
            buf,
        );
    }

    fn render_help(&self, full_area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let popup_w = 58u16.min(full_area.width.saturating_sub(4));
        let popup_h = 24u16.min(full_area.height.saturating_sub(4));
        let popup = centered_rect(popup_w, popup_h, full_area);

        ratatui::widgets::Widget::render(Clear, popup, buf);

        let content: Vec<Line<'static>> = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "  GLOBALES",
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled("  Tab / Shift+Tab  ", Style::default().fg(AMBER)),
                Span::styled("Foco siguiente/anterior", Style::default().fg(AMBER_DIM)),
            ]),
            Line::from(vec![
                Span::styled("  H                ", Style::default().fg(AMBER)),
                Span::styled("Esta pantalla de ayuda", Style::default().fg(AMBER_DIM)),
            ]),
            Line::from(vec![
                Span::styled("  Q / Ctrl+C       ", Style::default().fg(AMBER)),
                Span::styled("Salir", Style::default().fg(AMBER_DIM)),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+S           ", Style::default().fg(AMBER)),
                Span::styled("Guardar layout", Style::default().fg(AMBER_DIM)),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  REDIMENSIONAR WIDGET ACTIVO",
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled("  + / =            ", Style::default().fg(AMBER)),
                Span::styled("Ampliar ancho", Style::default().fg(AMBER_DIM)),
            ]),
            Line::from(vec![
                Span::styled("  -                ", Style::default().fg(AMBER)),
                Span::styled("Reducir ancho", Style::default().fg(AMBER_DIM)),
            ]),
            Line::from(vec![
                Span::styled("  }                ", Style::default().fg(AMBER)),
                Span::styled("Ampliar alto de fila", Style::default().fg(AMBER_DIM)),
            ]),
            Line::from(vec![
                Span::styled("  {                ", Style::default().fg(AMBER)),
                Span::styled("Reducir alto de fila", Style::default().fg(AMBER_DIM)),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  WIDGET: NOTICIAS (RSS)",
                Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled(
                    "  \u{2191} K / \u{2193} J        ",
                    Style::default().fg(AMBER),
                ),
                Span::styled("Navegar noticias", Style::default().fg(AMBER_DIM)),
            ]),
            Line::from(vec![
                Span::styled("  Enter            ", Style::default().fg(AMBER)),
                Span::styled("Ver detalle", Style::default().fg(AMBER_DIM)),
            ]),
            Line::from(vec![
                Span::styled("  L                ", Style::default().fg(AMBER)),
                Span::styled("Leer en voz alta (TTS)", Style::default().fg(AMBER_DIM)),
            ]),
            Line::from(vec![
                Span::styled("  Esc / Backspace  ", Style::default().fg(AMBER)),
                Span::styled("Volver / detener TTS", Style::default().fg(AMBER_DIM)),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  \u{2500}\u{2500} [ ESC o H para cerrar ] \u{2500}\u{2500}",
                Style::default().fg(AMBER_DIM),
            )]),
        ];

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Plain)
            .border_style(Style::default().fg(GREEN))
            .title(" AYUDA // SHORTCUTS ")
            .title_style(Style::default().fg(GREEN).add_modifier(Modifier::BOLD));

        let inner = block.inner(popup);
        ratatui::widgets::Widget::render(block, popup, buf);
        ratatui::widgets::Widget::render(
            Paragraph::new(Text::from(content)).wrap(Wrap { trim: false }),
            inner,
            buf,
        );
    }

    fn save_state(&self) {
        AppState::from_layout(self.layout.export_config()).save();
    }

    fn content_area(area: Rect) -> Rect {
        Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1))
    }

    fn statusbar_area(area: Rect) -> Rect {
        let y = area.y + area.height.saturating_sub(1);
        Rect::new(area.x, y, area.width, 1)
    }
}

// ──────────────────────────────────────────────────────────────

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

fn pseudo_rand(seed: u64) -> u64 {
    let mut x = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn glitch_title(title: &str, seed: u64) -> String {
    let chars: Vec<char> = title.chars().collect();
    let n = chars.len();
    if n == 0 {
        return title.to_string();
    }
    let mut out = chars;
    let i1 = (seed % n as u64) as usize;
    out[i1] = GLITCH_NOISE[(seed >> 4) as usize % GLITCH_NOISE.len()];
    if n > 6 {
        let i2 = (seed.wrapping_mul(7).wrapping_add(3) % n as u64) as usize;
        out[i2] = GLITCH_NOISE[(seed >> 12) as usize % GLITCH_NOISE.len()];
    }
    out.iter().collect()
}
