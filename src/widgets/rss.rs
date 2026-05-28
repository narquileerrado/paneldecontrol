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
    widgets::{Paragraph, Wrap},
};
use std::io::{Cursor, Write};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};
use tokio::task::AbortHandle;

struct RssEntry {
    title: String,
    summary: String,
}

pub struct RssWidget {
    id: WidgetId,
    url: String,
    max_items: usize,
    ttl: Duration,
    tts_lang: String,
    tts_cmd: String, // comando de shell; texto llega por stdin
    entries: Vec<RssEntry>,
    selected: usize,
    detail_view: bool,
    scroll: u16,
    last_fetch: Option<Instant>,
    worker: Option<AbortHandle>,
    tts_child: Option<Child>,
}

impl RssWidget {
    pub async fn init(config: WidgetConfig, _ctx: WidgetContext) -> Result<Box<dyn Widget>> {
        let url = config
            .params
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let max_items = config
            .params
            .get("max_items")
            .and_then(|v| v.as_integer())
            .unwrap_or(8) as usize;
        let ttl_secs = config
            .params
            .get("ttl_secs")
            .and_then(|v| v.as_integer())
            .unwrap_or(300) as u64;
        let tts_lang = config
            .params
            .get("tts_lang")
            .and_then(|v| v.as_str())
            .unwrap_or("es")
            .to_string();
        let tts_cmd = config
            .params
            .get("tts_cmd")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(Box::new(Self {
            id: config.id,
            url,
            max_items,
            ttl: Duration::from_secs(ttl_secs),
            tts_lang,
            tts_cmd,
            entries: Vec::new(),
            selected: 0,
            detail_view: false,
            scroll: 0,
            last_fetch: None,
            worker: None,
            tts_child: None,
        }))
    }

    // Inicia TTS del sistema. Detiene cualquier lectura en curso antes de empezar.
    fn speak(&mut self, text: &str) {
        self.stop_tts();

        let text = clean_for_tts(text);
        let text = truncate_chars(&text, 1200);

        if !self.tts_cmd.is_empty() {
            // Comando configurado por el usuario (ej: piper + aplay).
            // El texto se envía por stdin para no tener límites de longitud de arg
            // y para ser compatible con herramientas que no aceptan texto como arg.
            match std::process::Command::new("sh")
                .args(["-c", &self.tts_cmd])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = writeln!(stdin, "{}", text);
                        // stdin se descarta aquí → EOF → el proceso TTS termina solo
                    }
                    self.tts_child = Some(child);
                    return;
                }
                Err(e) => tracing::warn!(cmd = self.tts_cmd, error = %e, "error al lanzar tts_cmd"),
            }
        }

        // Fallback: auto-detección de TTS del sistema (texto como argumento)
        let lang = &self.tts_lang;
        if let Ok(child) = std::process::Command::new("spd-say")
            .args(["-l", lang, &text])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            self.tts_child = Some(child);
            return;
        }
        for cmd in ["espeak-ng", "espeak"] {
            if let Ok(child) = std::process::Command::new(cmd)
                .args(["-v", lang, &text])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                self.tts_child = Some(child);
                return;
            }
        }
        tracing::warn!(
            "TTS no disponible — configurar tts_cmd en config.toml o instalar espeak-ng/spd-say"
        );
    }

    fn stop_tts(&mut self) {
        if let Some(mut child) = self.tts_child.take() {
            match child.try_wait() {
                Ok(Some(_)) => {} // ya terminó, nada que matar
                _ => {
                    // Matar hijos del proceso sh (piper + aplay en la pipeline)
                    // antes de matar sh; si no, quedan huérfanos y siguen sonando.
                    let _ = std::process::Command::new("pkill")
                        .args(["-P", &child.id().to_string()])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
    }

    fn render_list(&self, area: Rect, buf: &mut Buffer) {
        const AMBER: Color = Color::Rgb(0xff, 0xb0, 0x00);
        const AMBER_DIM: Color = Color::Rgb(0x80, 0x58, 0x00);

        if self.entries.is_empty() {
            ratatui::widgets::Widget::render(
                Paragraph::new("Cargando...").style(Style::default().fg(AMBER_DIM)),
                area,
                buf,
            );
            return;
        }

        let height = area.height as usize;
        let scroll = (self.selected + 1).saturating_sub(height);

        for (i, entry) in self.entries.iter().enumerate().skip(scroll).take(height) {
            let style = if i == self.selected {
                Style::default().fg(AMBER).add_modifier(Modifier::REVERSED)
            } else {
                Style::default().fg(AMBER_DIM)
            };
            let line = format!("{}. {}", i + 1, entry.title);
            ratatui::widgets::Widget::render(
                Paragraph::new(line).style(style),
                Rect::new(area.x, area.y + (i - scroll) as u16, area.width, 1),
                buf,
            );
        }
    }

    fn render_detail(&self, area: Rect, buf: &mut Buffer) {
        const AMBER: Color = Color::Rgb(0xff, 0xb0, 0x00);

        let entry = &self.entries[self.selected];

        ratatui::widgets::Widget::render(
            Paragraph::new(entry.title.as_str())
                .style(Style::default().fg(AMBER).add_modifier(Modifier::BOLD)),
            Rect::new(area.x, area.y, area.width, 1),
            buf,
        );

        if area.height > 2 {
            let body = if entry.summary.is_empty() {
                "(sin resumen disponible)"
            } else {
                entry.summary.as_str()
            };
            ratatui::widgets::Widget::render(
                Paragraph::new(body)
                    .style(Style::default().fg(AMBER))
                    .wrap(Wrap { trim: false })
                    .scroll((self.scroll, 0)),
                Rect::new(area.x, area.y + 2, area.width, area.height - 2),
                buf,
            );
        }
    }
}

impl Widget for RssWidget {
    fn id(&self) -> &WidgetId {
        &self.id
    }
    fn kind(&self) -> &str {
        "rss"
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
        let url = self.url.clone();
        let max_items = self.max_items;
        let ttl = self.ttl;
        let tx = ctx.tx;
        let widget_id = ctx.widget_id;

        let handle = tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .user_agent("Mozilla/5.0 (compatible; paneldecontrol/0.1)")
                .build()
                .expect("HTTP client");

            let mut attempts: u32 = 0;
            loop {
                let msg = match fetch_rss(&client, &url, max_items).await {
                    Ok(entries) => {
                        attempts = 0;
                        WidgetMsg::Entries(entries)
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
        self.stop_tts();
    }

    fn update(&mut self, msg: WidgetMsg) {
        match msg {
            WidgetMsg::Entries(raw) => {
                self.entries = raw
                    .into_iter()
                    .map(|(title, summary)| RssEntry { title, summary })
                    .collect();
                self.last_fetch = Some(Instant::now());
                if self.selected >= self.entries.len() && !self.entries.is_empty() {
                    self.selected = self.entries.len() - 1;
                }
            }
            WidgetMsg::Error(e) => {
                tracing::warn!(id = self.id, error = e, "rss fetch error");
                if self.entries.is_empty() {
                    self.entries = vec![RssEntry {
                        title: format!("Error: {}", e),
                        summary: String::new(),
                    }];
                }
            }
            WidgetMsg::Lines(_) | WidgetMsg::Sistema(_) => {}
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if self.detail_view {
            self.render_detail(area, buf);
        } else {
            self.render_list(area, buf);
        }
    }

    fn handle_input(&mut self, ev: InputEvent) -> WidgetAction {
        let InputEvent::Key(key) = ev else {
            return WidgetAction::None;
        };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.detail_view {
                    self.scroll = self.scroll.saturating_sub(1);
                } else if self.selected > 0 {
                    self.selected -= 1;
                }
                WidgetAction::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.detail_view {
                    self.scroll += 1;
                } else if !self.entries.is_empty() && self.selected < self.entries.len() - 1 {
                    self.selected += 1;
                }
                WidgetAction::Consumed
            }
            KeyCode::Enter => {
                if !self.entries.is_empty() && !self.detail_view {
                    self.detail_view = true;
                    self.scroll = 0;
                    WidgetAction::Consumed
                } else {
                    WidgetAction::None
                }
            }
            KeyCode::Esc | KeyCode::Backspace => {
                if self.detail_view {
                    self.stop_tts();
                    self.detail_view = false;
                    WidgetAction::Consumed
                } else {
                    WidgetAction::None
                }
            }
            KeyCode::Char('l') => {
                if self.entries.is_empty() {
                    return WidgetAction::None;
                }
                let entry = &self.entries[self.selected];
                let text = if self.detail_view {
                    // Título + resumen completo (hasta 800 chars — se trunca en speak())
                    if entry.summary.is_empty() {
                        entry.title.clone()
                    } else {
                        format!("{}. {}", entry.title, entry.summary)
                    }
                } else {
                    entry.title.clone()
                };
                self.speak(&text);
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

async fn fetch_rss(
    client: &reqwest::Client,
    url: &str,
    max_items: usize,
) -> Result<Vec<(String, String)>> {
    if url.is_empty() {
        return Ok(vec![(
            "url no configurada en config.toml".to_string(),
            String::new(),
        )]);
    }
    let body = client.get(url).send().await?.bytes().await?;
    parse_feed(body.as_ref(), max_items)
}

fn parse_feed(xml: &[u8], max_items: usize) -> Result<Vec<(String, String)>> {
    let feed = feed_rs::parser::parse(Cursor::new(xml))?;
    let entries = feed
        .entries
        .iter()
        .take(max_items)
        .map(|e| {
            let title = e
                .title
                .as_ref()
                .map(|t| t.content.clone())
                .unwrap_or_else(|| "Sin título".to_string());

            let summary = e
                .content
                .as_ref()
                .and_then(|c| c.body.as_deref())
                .or_else(|| e.summary.as_ref().map(|s| s.content.as_str()))
                .map(strip_html)
                .unwrap_or_default();

            (title, summary)
        })
        .collect();
    Ok(entries)
}

fn strip_html(s: &str) -> String {
    // Decodificar entidades primero para que etiquetas como &lt;b&gt; también se limpien.
    let decoded = decode_html_entities(s);
    let mut out = String::with_capacity(decoded.len());
    let mut in_tag = false;
    for ch in decoded.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// Decodifica entidades HTML: numéricas (&#233; &#xE9;) y nombres comunes.
fn decode_html_entities(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        result.push_str(&rest[..amp]);
        rest = &rest[amp..];
        // Buscar el ';' que cierra la entidad (máx 32 chars para el nombre)
        if let Some(semi) = rest[1..].find(';').filter(|&p| p <= 30).map(|p| p + 1) {
            let entity = &rest[..semi + 1];
            if let Some(ch) = decode_entity(entity) {
                result.push(ch);
                rest = &rest[semi + 1..];
                continue;
            }
        }
        // No es una entidad reconocida: emitir el '&' literal y seguir.
        result.push('&');
        rest = &rest[1..];
    }
    result.push_str(rest);
    result
}

fn decode_entity(e: &str) -> Option<char> {
    // Entidades numéricas hex: &#xNN; o &#XNN;
    if e.len() > 4 && e.starts_with("&#") && (e.as_bytes()[2] == b'x' || e.as_bytes()[2] == b'X') {
        let hex = &e[3..e.len() - 1];
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
    }
    // Entidades numéricas decimales: &#NNN;
    if e.len() > 3 && e.starts_with("&#") {
        let dec = &e[2..e.len() - 1];
        return dec.parse::<u32>().ok().and_then(char::from_u32);
    }
    // Entidades nominales
    match e {
        "&amp;" => Some('&'),
        "&lt;" => Some('<'),
        "&gt;" => Some('>'),
        "&quot;" => Some('"'),
        "&apos;" => Some('\''),
        "&nbsp;" => Some('\u{00A0}'),
        // Vocales con acento (minúsculas)
        "&aacute;" => Some('á'),
        "&eacute;" => Some('é'),
        "&iacute;" => Some('í'),
        "&oacute;" => Some('ó'),
        "&uacute;" => Some('ú'),
        // Vocales con acento (mayúsculas)
        "&Aacute;" => Some('Á'),
        "&Eacute;" => Some('É'),
        "&Iacute;" => Some('Í'),
        "&Oacute;" => Some('Ó'),
        "&Uacute;" => Some('Ú'),
        // Eñe
        "&ntilde;" => Some('ñ'),
        "&Ntilde;" => Some('Ñ'),
        // Diéresis
        "&uuml;" => Some('ü'),
        "&Uuml;" => Some('Ü'),
        // Vocales con grave
        "&agrave;" => Some('à'),
        "&egrave;" => Some('è'),
        "&igrave;" => Some('ì'),
        "&ograve;" => Some('ò'),
        "&ugrave;" => Some('ù'),
        // Cedilla
        "&ccedil;" => Some('ç'),
        "&Ccedil;" => Some('Ç'),
        // Signos de puntuación españoles
        "&iexcl;" => Some('¡'),
        "&iquest;" => Some('¿'),
        "&laquo;" => Some('«'),
        "&raquo;" => Some('»'),
        // Tipografía
        "&mdash;" => Some('—'),
        "&ndash;" => Some('–'),
        "&ldquo;" => Some('"'),
        "&rdquo;" => Some('"'),
        "&lsquo;" => Some('\u{2018}'),
        "&rsquo;" => Some('\u{2019}'),
        "&hellip;" => Some('…'),
        "&bull;" => Some('•'),
        "&middot;" => Some('·'),
        // Símbolos
        "&copy;" => Some('©'),
        "&reg;" => Some('®'),
        "&trade;" => Some('™'),
        "&euro;" => Some('€'),
        "&pound;" => Some('£'),
        "&deg;" => Some('°'),
        "&plusmn;" => Some('±'),
        "&times;" => Some('×'),
        "&divide;" => Some('÷'),
        _ => None,
    }
}

// Convierte entidades HTML básicas a texto plano para que el TTS las lea bien.
fn clean_for_tts(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

// ──────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    const RSS_FIXTURE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Test Feed</title>
    <link>https://example.com</link>
    <description>Feed de prueba</description>
    <item>
      <title>Primera noticia</title>
      <link>https://example.com/1</link>
      <description>Resumen de la &lt;b&gt;primera&lt;/b&gt; noticia.</description>
    </item>
    <item>
      <title>Segunda noticia</title>
      <link>https://example.com/2</link>
      <description>Resumen de la segunda noticia.</description>
    </item>
    <item>
      <title>Tercera noticia</title>
      <link>https://example.com/3</link>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn parsea_titulos_correctamente() {
        let entries = parse_feed(RSS_FIXTURE, 10).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].0, "Primera noticia");
        assert_eq!(entries[1].0, "Segunda noticia");
        assert_eq!(entries[2].0, "Tercera noticia");
    }

    #[test]
    fn parsea_resumen_y_strip_html() {
        let entries = parse_feed(RSS_FIXTURE, 10).unwrap();
        assert!(
            entries[0].1.contains("primera"),
            "debe contener el texto del resumen"
        );
        assert!(
            !entries[0].1.contains('<'),
            "no debe contener etiquetas HTML"
        );
    }

    #[test]
    fn respeta_max_items() {
        let entries = parse_feed(RSS_FIXTURE, 2).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].0, "Segunda noticia");
    }

    #[test]
    fn xml_invalido_retorna_error() {
        assert!(parse_feed(b"esto no es xml valido", 10).is_err());
    }

    #[test]
    fn strip_html_basico() {
        assert_eq!(strip_html("<b>Hola</b> <i>mundo</i>"), "Hola mundo");
        assert_eq!(strip_html("sin tags"), "sin tags");
        assert_eq!(strip_html(""), "");
    }

    #[test]
    fn decode_entidades_named() {
        assert_eq!(
            decode_html_entities("El caf&eacute; est&aacute; listo"),
            "El café está listo"
        );
        assert_eq!(decode_html_entities("&ntilde;o&ntilde;o"), "ñoño");
        assert_eq!(decode_html_entities("&iquest;Cu&aacute;ndo?"), "¿Cuándo?");
    }

    #[test]
    fn decode_entidades_numericas() {
        assert_eq!(decode_html_entities("caf&#233;"), "café"); // decimal
        assert_eq!(decode_html_entities("&#xF1;o"), "ño"); // hex
        assert_eq!(decode_html_entities("&#X41;"), "A"); // hex mayúscula
    }

    #[test]
    fn strip_html_decodifica_entidades() {
        assert_eq!(
            strip_html("El caf&eacute; &amp; <b>el t&eacute;</b>"),
            "El café & el té"
        );
        // Etiquetas codificadas como entidades también se eliminan
        assert_eq!(strip_html("&lt;b&gt;texto&lt;/b&gt;"), "texto");
    }

    #[test]
    fn entidad_desconocida_se_preserva() {
        assert_eq!(decode_html_entities("&nada;"), "&nada;");
        assert_eq!(decode_html_entities("R&amp;B"), "R&B");
    }

    #[test]
    fn clean_for_tts_entidades() {
        assert_eq!(clean_for_tts("café &amp; vino"), "café & vino");
        assert_eq!(clean_for_tts("&quot;hola&quot;"), "\"hola\"");
        assert_eq!(clean_for_tts("sin entidades"), "sin entidades");
    }

    #[test]
    fn truncate_chars_respeta_unicode() {
        let s = "áéíóú";
        assert_eq!(truncate_chars(s, 3), "áéí");
        assert_eq!(truncate_chars(s, 10), "áéíóú");
    }
}
