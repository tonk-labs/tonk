use std::time::Duration;

use ratatui::{
    DefaultTerminal,
    crossterm::event::{self, Event, KeyEvent, KeyEventKind},
    prelude::*,
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TonkCliInspectorError {
    #[error("Could not initialize the inspector: {0}")]
    Initialization(String),

    #[error("Could not read key input: {0}")]
    KeyInput(String),

    #[error("Could not update the terminal UI: {0}")]
    Update(String),
}

#[derive(Default)]
pub struct TonkCliInspectorState {
    should_exit: bool,
    key_event: Option<KeyEvent>,
}

pub struct TonkCliInspector {
    state: TonkCliInspectorState,
}

impl TonkCliInspector {
    pub fn new(state: TonkCliInspectorState) -> Self {
        Self { state }
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> Result<(), TonkCliInspectorError> {
        loop {
            if self.state.should_exit {
                break;
            }

            terminal
                .draw(|_frame| {

                    // frame.render_stateful_widget(&DiagnoseApp {}, frame.area(), &mut self.state)
                })
                .map_err(|error| TonkCliInspectorError::Update(format!("{error}")))?;

            self.handle_events()?;
        }
        Ok(())
    }

    fn handle_events(&mut self) -> Result<(), TonkCliInspectorError> {
        self.state.key_event = None;

        if event::poll(Duration::from_millis(100))
            .map_err(|error| TonkCliInspectorError::KeyInput(format!("{error}")))?
        {
            match event::read()
                .map_err(|error| TonkCliInspectorError::KeyInput(format!("{error}")))?
            {
                // it's important to check that the event is a key press event as
                // crossterm also emits key release and repeat events on Windows.
                Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                    self.state.key_event = Some(key_event);
                }
                _ => {}
            };
        }

        Ok(())
    }
}

pub async fn start_inspector() -> Result<(), TonkCliInspectorError> {
    let mut terminal = ratatui::init();

    terminal
        .clear()
        .map_err(|error| TonkCliInspectorError::Initialization(format!("{error}")))?;

    let inspector = TonkCliInspector::new(TonkCliInspectorState::default());

    inspector.run(terminal).await?;

    ratatui::restore();

    Ok(())
}
