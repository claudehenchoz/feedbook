use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Arc, Mutex};

pub type LogFile = Arc<Mutex<BufWriter<File>>>;

#[derive(Clone)]
pub struct LogSink {
    prefix: String,
    file: Option<LogFile>,
}

impl LogSink {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self { prefix: prefix.into(), file: None }
    }

    pub fn with_file_opt(mut self, file: Option<LogFile>) -> Self {
        self.file = file;
        self
    }

    pub fn println(&self, msg: &str) {
        if self.prefix.is_empty() {
            println!("{}", msg);
        } else {
            println!("{}: {}", self.prefix, msg);
        }
        if let Some(f) = &self.file {
            if let Ok(mut f) = f.lock() {
                let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                if self.prefix.is_empty() {
                    let _ = writeln!(f, "[{ts}] {msg}");
                } else {
                    let _ = writeln!(f, "[{ts}] {}: {msg}", self.prefix);
                }
            }
        }
    }
}
