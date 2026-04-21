use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::time::LocalTime;
use tracing_subscriber::{EnvFilter, fmt};

/// Logging configuration extracted from server/client configs.
pub struct LogConfig<'a> {
    /// Log level string, e.g. "info", "debug", "warn". Defaults to "info".
    pub log_level: &'a str,
    /// Optional path to a log file. If empty, logs go to stdout.
    /// When set, the directory portion is used as the directory and the
    /// filename portion (without extension) is used as the file prefix.
    /// Files are rotated daily: `<prefix>.YYYY-MM-DD`.
    pub log_file: &'a str,
    /// How many days of log files to keep. 0 means keep forever. Default: 1.
    pub log_max_days: u32,
}

/// Initialise the global tracing subscriber.
///
/// Returns an optional [`WorkerGuard`] that **must** be kept alive for the
/// duration of the process. Dropping it flushes and closes the background
/// writer thread.  When logging to stdout the guard is `None`.
pub fn init_logging(cfg: LogConfig<'_>) -> Option<WorkerGuard> {
    let level = if cfg.log_level.is_empty() {
        "info"
    } else {
        cfg.log_level
    };

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    if cfg.log_file.is_empty() {
        // ── stdout (default) ──────────────────────────────────────────────
        fmt()
            .with_env_filter(filter)
            .with_timer(LocalTime::rfc_3339())
            .init();
        None
    } else {
        // ── file + daily rotation ─────────────────────────────────────────
        let log_path = Path::new(cfg.log_file);
        let dir = log_path.parent().unwrap_or_else(|| Path::new("."));
        let prefix = log_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("app");

        // Create the directory if it doesn't exist.
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("Failed to create log directory {:?}: {}", dir, e);
        }

        // tracing-appender daily rotation: files named `<prefix>.YYYY-MM-DD`.
        let file_appender =
            tracing_appender::rolling::daily(dir, prefix);

        // Purge old files beyond log_max_days (best effort, ignore errors).
        if cfg.log_max_days > 0 {
            purge_old_logs(dir, prefix, cfg.log_max_days);
        }

        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

        fmt()
            .with_env_filter(filter)
            .with_timer(LocalTime::rfc_3339())
            .with_ansi(false)      // no ANSI colour codes in file logs
            .with_writer(non_blocking)
            .init();

        Some(guard)
    }
}

/// Delete log files older than `max_days` in `dir` whose name starts with
/// `prefix`.  Called once at startup; errors are silently ignored.
fn purge_old_logs(dir: &Path, prefix: &str, max_days: u32) {
    let cutoff = chrono::Utc::now()
        - chrono::Duration::days(max_days as i64);

    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };

    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Only touch files whose name starts with our prefix.
        if !name_str.starts_with(prefix) {
            continue;
        }

        // tracing-appender names files `<prefix>.YYYY-MM-DD`.
        // Extract the date suffix after the last '.'.
        let date_str = match name_str.rfind('.') {
            Some(pos) => &name_str[pos + 1..],
            None => continue,
        };

        // Parse YYYY-MM-DD.
        let file_date =
            match chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                Ok(d) => d,
                Err(_) => continue,
            };

        let file_dt = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
            file_date.and_hms_opt(0, 0, 0).unwrap(),
            chrono::Utc,
        );

        if file_dt < cutoff {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
