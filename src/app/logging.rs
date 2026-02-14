use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::PathBuf,
    sync::mpsc,
    thread,
};

use chrono::Local;
use colored::Colorize;
use log::{Level, LevelFilter, Metadata, Record};
use tokio::fs;

#[derive(Debug)]
struct AsyncLogEvent {
    timestamp: String,
    level: Level,
    message: String,
}

#[derive(Debug)]
struct AsyncLogger {
    level_filter: LevelFilter,
    sender: mpsc::Sender<AsyncLogEvent>,
}

impl AsyncLogger {
    const fn new(level_filter: LevelFilter, sender: mpsc::Sender<AsyncLogEvent>) -> Self {
        Self {
            level_filter,
            sender,
        }
    }
}

#[derive(Debug)]
struct LogWriter {
    receiver: mpsc::Receiver<AsyncLogEvent>,
    log_path: PathBuf,
}

impl LogWriter {
    const fn new(receiver: mpsc::Receiver<AsyncLogEvent>, log_path: PathBuf) -> Self {
        Self { receiver, log_path }
    }

    fn run(self) {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path);
        let mut file_writer = match file {
            Ok(file) => Some(BufWriter::new(file)),
            Err(error) => {
                eprintln!(
                    "Failed to open log file '{}': {}",
                    self.log_path.display(),
                    error
                );
                None
            }
        };

        let stdout = std::io::stdout();
        let mut stdout_lock = stdout.lock();

        while let Ok(event) = self.receiver.recv() {
            let console_level = colored_level(event.level);
            let file_level = plain_level(event.level);

            if let Err(error) = writeln!(
                stdout_lock,
                "{} [ {} ] > {}",
                event.timestamp, console_level, event.message
            ) {
                eprintln!("Failed to write log line to stdout: {}", error);
            }

            if let Some(writer) = file_writer.as_mut() {
                if let Err(error) = writeln!(
                    writer,
                    "{} [ {} ] > {}",
                    event.timestamp, file_level, event.message
                ) {
                    eprintln!("Failed to write log line to file: {}", error);
                    file_writer = None;
                    continue;
                }

                if let Err(error) = writer.flush() {
                    eprintln!("Failed to flush log file writer: {}", error);
                    file_writer = None;
                }
            }
        }
    }
}

impl log::Log for AsyncLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level_filter
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let event = AsyncLogEvent {
            timestamp: Local::now().format("%Y-%m-%d %H:%M:%S.%3f").to_string(),
            level: record.level(),
            message: format!("{}", record.args()),
        };

        if let Err(_error) = self.sender.send(event) {
            // Logger channel is down; avoid recursive logging.
        }
    }

    fn flush(&self) {}
}

pub async fn init_logging(level_filter: LevelFilter) -> Result<(), String> {
    let log_dir = PathBuf::from("log");
    fs::create_dir_all(&log_dir)
        .await
        .map_err(|error| format!("Failed to create log directory: {}", error))?;

    let log_path = log_dir.join("output.ans");
    let (sender, receiver) = mpsc::channel::<AsyncLogEvent>();
    let writer = LogWriter::new(receiver, log_path);

    thread::spawn(move || {
        writer.run();
    });

    let logger = AsyncLogger::new(level_filter, sender);
    if let Err(_error) = log::set_boxed_logger(Box::new(logger)) {
        // Logger may already be initialized in process lifecycle.
        log::set_max_level(level_filter);
        return Ok(());
    }

    log::set_max_level(level_filter);
    Ok(())
}

fn colored_level(level: Level) -> String {
    match level {
        Level::Info => "+".green().to_string(),
        Level::Error => "-".red().to_string(),
        Level::Warn => "!".yellow().to_string(),
        Level::Debug => "*".blue().to_string(),
        Level::Trace => "~".purple().to_string(),
    }
}

const fn plain_level(level: Level) -> &'static str {
    match level {
        Level::Info => "+",
        Level::Error => "-",
        Level::Warn => "!",
        Level::Debug => "*",
        Level::Trace => "~",
    }
}
