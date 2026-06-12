use std::path::Path;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init_logging(logs_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let app_log = logs_dir.join("app.log");
    let app_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(app_log)?;

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(app_file)
        .with_target(true);

    let stdout_layer = fmt::layer().with_target(false);

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(stdout_layer)
        .with(file_layer)
        .init();

    Ok(())
}

pub fn append_log(logs_dir: &Path, filename: &str, line: &str) {
    let path = logs_dir.join(filename);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = writeln!(file, "{line}");
    }
}

pub fn redact_secrets(message: &str, secrets: &[&str]) -> String {
    let mut result = message.to_string();
    for secret in secrets {
        if !secret.is_empty() {
            result = result.replace(secret, "***REDACTED***");
        }
    }
    result
}
