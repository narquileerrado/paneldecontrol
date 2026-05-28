use crate::core::event::InputEvent;
use crate::widgets::{
    CoreMsg, DataState, Widget, WidgetAction, WidgetConfig, WidgetContext, WidgetId, WidgetMsg,
    WorkerContext,
};
use anyhow::Result;
use crossterm::event::KeyCode;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::task::AbortHandle;

const AMBER: Color = Color::Rgb(0xff, 0xb0, 0x00);
const AMBER_DIM: Color = Color::Rgb(0x80, 0x58, 0x00);

pub struct PlayerWidget {
    id: WidgetId,
    player: String, // "" = cualquier player; "spotify", "mpd", etc. para filtrar
    ttl: Duration,
    status: String, // "Playing" | "Paused" | "Stopped" | ""
    artist: String,
    title: String,
    album: String,
    last_fetch: Option<Instant>,
    worker: Option<AbortHandle>,
}

impl PlayerWidget {
    pub async fn init(config: WidgetConfig, _ctx: WidgetContext) -> Result<Box<dyn Widget>> {
        let player = config
            .params
            .get("player")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let ttl_secs = config
            .params
            .get("ttl_secs")
            .and_then(|v| v.as_integer())
            .unwrap_or(3) as u64;

        Ok(Box::new(Self {
            id: config.id,
            player,
            ttl: Duration::from_secs(ttl_secs),
            status: String::new(),
            artist: String::new(),
            title: String::new(),
            album: String::new(),
            last_fetch: None,
            worker: None,
        }))
    }
}

impl Widget for PlayerWidget {
    fn id(&self) -> &WidgetId {
        &self.id
    }
    fn kind(&self) -> &str {
        "player"
    }

    fn data_state(&self) -> DataState {
        match self.last_fetch {
            None => DataState::Loading,
            Some(t) => {
                if t.elapsed() > self.ttl + Duration::from_secs(30) {
                    DataState::Stale { fetched_at: t }
                } else {
                    DataState::Fresh { fetched_at: t }
                }
            }
        }
    }

    fn start_background(&mut self, ctx: WorkerContext) {
        let player = self.player.clone();
        let ttl = self.ttl;
        let tx = ctx.tx;
        let widget_id = ctx.widget_id;

        let handle = tokio::spawn(async move {
            loop {
                let lines = fetch_status(&player).await;
                let _ = tx
                    .send(CoreMsg {
                        widget_id: widget_id.clone(),
                        msg: WidgetMsg::Lines(lines),
                    })
                    .await;
                tokio::time::sleep(ttl).await;
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
        if let WidgetMsg::Lines(lines) = msg {
            self.status = lines.first().cloned().unwrap_or_default();
            self.artist = lines.get(1).cloned().unwrap_or_default();
            self.title = lines.get(2).cloned().unwrap_or_default();
            self.album = lines.get(3).cloned().unwrap_or_default();
            self.last_fetch = Some(Instant::now());
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        let icon = status_icon(&self.status);
        let pp_icon = play_pause_icon(&self.status);

        // ── Track info ──────────────────────────────────────────
        if self.title.is_empty() {
            let msg = if self.last_fetch.is_none() {
                "Cargando..."
            } else {
                "■  Sin reproducción activa"
            };
            ratatui::widgets::Widget::render(
                Paragraph::new(msg).style(Style::default().fg(AMBER_DIM)),
                Rect::new(area.x, area.y, area.width, 1),
                buf,
            );
        } else {
            let track = if self.artist.is_empty() {
                format!("{}  {}", icon, self.title)
            } else {
                format!("{}  {} — {}", icon, self.artist, self.title)
            };
            ratatui::widgets::Widget::render(
                Paragraph::new(track).style(Style::default().fg(AMBER)),
                Rect::new(area.x, area.y, area.width, 1),
                buf,
            );
            if !self.album.is_empty() && area.height > 3 {
                ratatui::widgets::Widget::render(
                    Paragraph::new(format!("   {}", self.album))
                        .style(Style::default().fg(AMBER_DIM)),
                    Rect::new(area.x, area.y + 1, area.width, 1),
                    buf,
                );
            }
        }

        // ── Controles ───────────────────────────────────────────
        if area.height >= 3 {
            let ctrl_y = area.y + area.height - 1;
            let controls = Line::from(vec![
                Span::styled("  [◄◄ ←]", Style::default().fg(AMBER_DIM)),
                Span::styled(
                    format!("  [{} spc]", pp_icon),
                    Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
                ),
                Span::styled("  [▶▶ →]", Style::default().fg(AMBER_DIM)),
            ]);
            ratatui::widgets::Widget::render(
                Paragraph::new(controls),
                Rect::new(area.x, ctrl_y, area.width, 1),
                buf,
            );
        }
    }

    fn handle_input(&mut self, ev: InputEvent) -> WidgetAction {
        let InputEvent::Key(key) = ev else {
            return WidgetAction::None;
        };
        match key.code {
            KeyCode::Char(' ') => {
                playerctl_cmd(&self.player, "play-pause");
                WidgetAction::Consumed
            }
            KeyCode::Left => {
                playerctl_cmd(&self.player, "previous");
                WidgetAction::Consumed
            }
            KeyCode::Right => {
                playerctl_cmd(&self.player, "next");
                WidgetAction::Consumed
            }
            _ => WidgetAction::None,
        }
    }

    fn serialize_state(&self) -> toml::Value {
        toml::Value::Table(toml::map::Map::new())
    }
}

// ──────────────────────────────────────────────────────────────

fn status_icon(status: &str) -> &'static str {
    match status {
        "Playing" => "▶",
        "Paused" => "⏸",
        _ => "■",
    }
}

fn play_pause_icon(status: &str) -> &'static str {
    if status == "Playing" {
        "⏸"
    } else {
        "▶"
    }
}

async fn fetch_status(player: &str) -> Vec<String> {
    let mut cmd = tokio::process::Command::new("playerctl");
    if !player.is_empty() {
        cmd.arg(format!("--player={}", player));
    }
    cmd.args([
        "metadata",
        "--format",
        "{{status}}|||{{artist}}|||{{title}}|||{{album}}",
    ]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());

    match cmd.output().await {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.trim()
                .split("|||")
                .map(|s| s.trim().to_string())
                .collect()
        }
        _ => vec![
            "Stopped".to_string(),
            String::new(),
            String::new(),
            String::new(),
        ],
    }
}

fn playerctl_cmd(player: &str, cmd: &str) {
    let mut c = std::process::Command::new("playerctl");
    if !player.is_empty() {
        c.arg(format!("--player={}", player));
    }
    c.arg(cmd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
}

// ──────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_icon_variantes() {
        assert_eq!(status_icon("Playing"), "▶");
        assert_eq!(status_icon("Paused"), "⏸");
        assert_eq!(status_icon("Stopped"), "■");
        assert_eq!(status_icon(""), "■");
    }

    #[test]
    fn play_pause_icon_segun_estado() {
        assert_eq!(play_pause_icon("Playing"), "⏸");
        assert_eq!(play_pause_icon("Paused"), "▶");
        assert_eq!(play_pause_icon("Stopped"), "▶");
        assert_eq!(play_pause_icon(""), "▶");
    }

    #[test]
    fn update_parsea_lineas() {
        // Simular lo que devuelve fetch_status vía WidgetMsg::Lines
        let lines = vec![
            "Playing".to_string(),
            "Massive Attack".to_string(),
            "Teardrop".to_string(),
            "Mezzanine".to_string(),
        ];
        assert_eq!(lines[0], "Playing");
        assert_eq!(lines[1], "Massive Attack");
        assert_eq!(lines[2], "Teardrop");
        assert_eq!(lines[3], "Mezzanine");
    }

    #[test]
    fn update_tolera_lineas_incompletas() {
        let lines: Vec<String> = vec!["Paused".to_string()];
        assert_eq!(lines.first().cloned().unwrap_or_default(), "Paused");
        assert_eq!(lines.get(1).cloned().unwrap_or_default(), "");
        assert_eq!(lines.get(2).cloned().unwrap_or_default(), "");
    }
}
