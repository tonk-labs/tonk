//! The event loop: the only code here that needs a tty.
//!
//! It does three things, and deliberately no more — put the terminal in
//! raw mode, translate a crossterm key into a [`Key`], and print the
//! frame the session paints. Every decision that could be *wrong* lives
//! in `session`, which runs with no terminal at all, so this file is the
//! part a test cannot reach and also the part with nothing in it to get
//! wrong (`plan/tui-views.md` §12).
//!
//! Repainting is a full frame written over the alternate screen rather
//! than a damage-tracked diff. That is the honest thing at this size: a
//! frame is a `String` the painter already builds, and diffing it would
//! be optimising a loop that redraws only on a keypress.

use std::io::Write;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::{cursor, event, execute, terminal};

use crate::cli::{Options, prepare};
use crate::session::{Direction, Effect, Key};

/// Take over the terminal and run until the view quits.
///
/// Posted transients are collected rather than sent: this proof of
/// concept has no branch to transact against, and printing them on exit
/// is what makes the write path inspectable without inventing a fake
/// one.
pub fn run(options: &Options) -> Result<String, String> {
    let mut prepared = prepare(options)?;
    let mut posted: Vec<String> = Vec::new();

    let mut out = std::io::stdout();
    terminal::enable_raw_mode().map_err(|error| error.to_string())?;
    execute!(out, terminal::EnterAlternateScreen, cursor::Hide)
        .map_err(|error| error.to_string())?;

    // From here on every exit path has to restore the terminal, so the
    // loop's own error is captured rather than returned: leaving a shell
    // in raw mode with no cursor is a worse failure than whatever went
    // wrong inside.
    let outcome = (|| -> Result<(), String> {
        loop {
            draw(&mut out, &prepared.frame())?;
            let Some(key) = next_key()? else {
                continue;
            };
            match prepared.press(key) {
                Effect::Quit => return Ok(()),
                Effect::Post(body) => {
                    posted.push(serde_json::to_string(&body).map_err(|e| e.to_string())?)
                }
                Effect::Declined => posted.push("declined".to_string()),
                Effect::Idle | Effect::Moved => {}
            }
        }
    })();

    execute!(out, cursor::Show, terminal::LeaveAlternateScreen)
        .map_err(|error| error.to_string())?;
    terminal::disable_raw_mode().map_err(|error| error.to_string())?;
    outcome?;

    Ok(posted.join("\n"))
}

fn draw(out: &mut std::io::Stdout, frame: &str) -> Result<(), String> {
    execute!(
        out,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0)
    )
    .map_err(|error| error.to_string())?;
    // Raw mode does not translate `\n`, so each line needs its own
    // carriage return or the frame walks diagonally off the screen.
    for line in frame.lines() {
        write!(out, "{line}\r\n").map_err(|error| error.to_string())?;
    }
    out.flush().map_err(|error| error.to_string())
}

/// Block for the next key, or `None` for an event that is not one.
fn next_key() -> Result<Option<Key>, String> {
    match event::read().map_err(|error| error.to_string())? {
        // A key *release* is a separate event on terminals that report
        // one. Acting on both would fire every binding twice.
        Event::Key(key) if key.kind == KeyEventKind::Press => Ok(translate(key)),
        _ => Ok(None),
    }
}

fn translate(key: KeyEvent) -> Option<Key> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::BackTab => Some(Key::BackTab),
        KeyCode::Up => Some(Key::Arrow(Direction::Up)),
        KeyCode::Down => Some(Key::Arrow(Direction::Down)),
        KeyCode::Left => Some(Key::Arrow(Direction::Left)),
        KeyCode::Right => Some(Key::Arrow(Direction::Right)),
        KeyCode::Enter | KeyCode::Char(' ') => Some(Key::Activate),
        KeyCode::Esc => Some(Key::Quit),
        // `Ctrl-C` leaves, because raw mode means the terminal no longer
        // does it for us and a view with no `q` would otherwise be a
        // trap.
        KeyCode::Char('c') if control => Some(Key::Quit),
        KeyCode::Char(character) if !control => Some(Key::Char(character)),
        _ => None,
    }
}
