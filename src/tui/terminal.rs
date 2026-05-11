//! Crossterm + ratatui terminal setup with RAII restore on drop.

use std::io::{self, stdout};
use std::mem::ManuallyDrop;

use crossterm::{
    cursor::{Hide, Show},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

pub type TuiTerminal = Terminal<CrosstermBackend<io::Stdout>>;

/// Owns raw mode + alternate screen; restores terminal on drop.
pub struct TerminalGuard {
    inner: ManuallyDrop<TuiTerminal>,
}

impl TerminalGuard {
    pub fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut out = stdout();
        execute!(out, EnterAlternateScreen, EnableMouseCapture, Hide,)?;
        let backend = CrosstermBackend::new(out);
        let inner = Terminal::new(backend)?;
        Ok(Self {
            inner: ManuallyDrop::new(inner),
        })
    }

    pub fn terminal(&mut self) -> &mut TuiTerminal {
        &mut self.inner
    }

    /// Releases raw mode and alternate screen, then returns the underlying terminal.
    /// Skips [`Drop`] so the caller owns the [`TuiTerminal`] (e.g. for tests); the terminal
    /// is left in a restored TTY state consistent with [`Drop`].
    pub fn into_inner(mut self) -> TuiTerminal {
        let _ = self.inner.flush();
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableMouseCapture,
            LeaveAlternateScreen,
            Show,
        );
        let _ = self.inner.show_cursor();
        let inner = unsafe { ManuallyDrop::take(&mut self.inner) };
        std::mem::forget(self);
        inner
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.inner.flush();
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableMouseCapture,
            LeaveAlternateScreen,
            Show,
        );
        let _ = self.inner.show_cursor();
        unsafe {
            ManuallyDrop::drop(&mut self.inner);
        }
    }
}
