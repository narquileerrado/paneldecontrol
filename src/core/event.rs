use crate::widgets::CoreMsg;
use crossterm::event::{Event, EventStream, KeyEvent, MouseEvent};
use futures::StreamExt;
use std::time::Duration;
use tokio::{sync::mpsc, time};

pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Tick,
}

pub enum AppEvent {
    Input(InputEvent),
    Core(CoreMsg),
}

pub struct EventHandler {
    tick_rate: Duration,
    stream: EventStream,
    core_rx: mpsc::Receiver<CoreMsg>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration, core_rx: mpsc::Receiver<CoreMsg>) -> Self {
        Self {
            tick_rate,
            stream: EventStream::new(),
            core_rx,
        }
    }

    pub async fn next(&mut self) -> anyhow::Result<AppEvent> {
        tokio::select! {
            _ = time::sleep(self.tick_rate) => {
                Ok(AppEvent::Input(InputEvent::Tick))
            }
            maybe = self.stream.next() => match maybe {
                Some(Ok(Event::Key(k)))       => Ok(AppEvent::Input(InputEvent::Key(k))),
                Some(Ok(Event::Mouse(m)))     => Ok(AppEvent::Input(InputEvent::Mouse(m))),
                Some(Ok(Event::Resize(w, h))) => Ok(AppEvent::Input(InputEvent::Resize(w, h))),
                Some(Ok(_))                   => Ok(AppEvent::Input(InputEvent::Tick)),
                Some(Err(e))                  => Err(e.into()),
                None                          => Ok(AppEvent::Input(InputEvent::Tick)),
            },
            maybe = self.core_rx.recv() => match maybe {
                Some(msg) => Ok(AppEvent::Core(msg)),
                None      => Ok(AppEvent::Input(InputEvent::Tick)),
            }
        }
    }
}
