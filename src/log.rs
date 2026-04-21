use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Arc, Mutex};
use indicatif::ProgressBar;

pub type LogFile = Arc<Mutex<BufWriter<File>>>;

#[derive(Clone)]
pub struct LogSink {
    inner: SinkInner,
    file: Option<LogFile>,
}

#[derive(Clone)]
enum SinkInner {
    Bar(ProgressBar),
    Stderr,
}

impl LogSink {
    pub fn bar(pb: ProgressBar) -> Self {
        Self { inner: SinkInner::Bar(pb), file: None }
    }

    pub fn stderr() -> Self {
        Self { inner: SinkInner::Stderr, file: None }
    }

    pub fn with_file_opt(mut self, file: Option<LogFile>) -> Self {
        self.file = file;
        self
    }

    pub fn println(&self, msg: &str) {
        match &self.inner {
            SinkInner::Bar(pb) => pb.println(msg),
            SinkInner::Stderr  => eprintln!("{}", msg),
        }
        self.write_file(msg);
    }

    /// Write to the log file only — used when a progress bar already shows the message on screen.
    pub fn write_file(&self, msg: &str) {
        if let Some(f) = &self.file {
            if let Ok(mut f) = f.lock() {
                let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                let _ = writeln!(f, "[{ts}] {msg}");
            }
        }
    }

}
