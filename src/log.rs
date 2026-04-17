use indicatif::ProgressBar;

#[derive(Clone)]
pub enum LogSink {
    Bar(ProgressBar),
    Stderr,
}

impl LogSink {
    pub fn println(&self, msg: &str) {
        match self {
            Self::Bar(pb) => pb.println(msg),
            Self::Stderr => eprintln!("{}", msg),
        }
    }
}
