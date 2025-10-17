// vim: tw=80
use std::{
    net::{IpAddr, SocketAddr},
    process::exit,
    sync::{Arc, LazyLock},
};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use clap::{
    error::{Error as ClapError, ErrorKind as ClapErrorKind},
    Parser,
};
use env_logger::{Builder, Env};
use freebsd_libgeom::{Snapshot, Statistics, Tree};
use prometheus::{register_gauge_vec, GaugeVec, TextEncoder};
use regex::Regex;
use tokio::net::TcpListener;

/// Export GEOM device metrics to Prometheus
#[derive(Debug, Default, clap::Parser)]
struct Cli {
    /// Bind to this local address
    #[clap(short = 'b', default_value = "0.0.0.0")]
    addr:     String,
    /// Only report physical providers (those with rank of 1).
    #[clap(short = 'P', long = "physical")]
    physical: bool,
    /// Only report devices with names matching this regex.
    #[clap(short = 'f', long = "include", value_parser = regex_parser)]
    include:  Option<Regex>,
    /// Do not report devices with names matching this regex
    #[clap(short = 'F', long = "exclude", value_parser = regex_parser)]
    exclude:  Option<Regex>,
    /// TCP port
    #[clap(short = 'p', default_value = "9248")]
    port:     u16,
}

fn regex_parser(s: &str) -> Result<Regex, ClapError> {
    match Regex::new(s) {
        Ok(s) => Ok(s),
        Err(e) => Err(ClapError::raw(ClapErrorKind::ValueValidation, e)),
    }
}

static BUSY_TIME: LazyLock<GaugeVec> = LazyLock::new(|| {
    register_gauge_vec!(
        "geom_busy_time",
        "Cumulative time in seconds that the device had at least one \
         outstanding operation",
        &["device"],
    )
    .expect("cannot create gauge")
});
static BYTES: LazyLock<GaugeVec> = LazyLock::new(|| {
    register_gauge_vec!(
        "geom_bytes",
        "Total bytes processed",
        &["device", "method"]
    )
    .expect("cannot create gauge")
});
static DURATION: LazyLock<GaugeVec> = LazyLock::new(|| {
    register_gauge_vec!(
        "geom_duration",
        "Total time spent processing commands in seconds",
        &["device", "method"]
    )
    .expect("cannot create gauge")
});
static OPS: LazyLock<GaugeVec> = LazyLock::new(|| {
    register_gauge_vec!(
        "geom_operations",
        "Total operations processed",
        &["device", "method"]
    )
    .expect("cannot create gauge")
});
static QUEUE_LENGTH: LazyLock<GaugeVec> = LazyLock::new(|| {
    register_gauge_vec!(
        "geom_queue_length",
        "Number of incomplete transactions at the sampling instant",
        &["device"]
    )
    .expect("cannot create gauge")
});

/// Wrapper type that implements IntoResponse for anyhow::Error.
#[derive(Debug)]
struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("{}", self.0))
            .into_response()
    }
}

async fn metrics(cli: State<Arc<Cli>>) -> Result<String, AppError> {
    // inner relies on an implicit Into conversion to return anyhow::Error
    let inner = || -> Result<String, anyhow::Error> {
        // Note: it might be more efficient to only call Tree:new if we detect
        // that a device has arrived or departed.  But on a system with hundreds
        // of disks, it only takes 13ms.
        let mut tree = Tree::new()?;
        let mut current = Snapshot::new()?;
        BUSY_TIME.reset();

        for item in current.iter() {
            if let Some(gident) = tree.lookup(item.id()) {
                if let Some(rank) = gident.rank() {
                    if rank > 1 && cli.physical {
                        continue;
                    }
                    let device = gident.name().unwrap().to_string_lossy();
                    if !cli
                        .include
                        .as_ref()
                        .map(|f| f.is_match(&device))
                        .unwrap_or(true)
                    {
                        continue;
                    }
                    if cli
                        .exclude
                        .as_ref()
                        .map(|f| f.is_match(&device))
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    let stats = Statistics::compute(item, None, 0.0);

                    BUSY_TIME
                        .with_label_values(&[&device])
                        .set(stats.busy_time());
                    QUEUE_LENGTH
                        .with_label_values(&[&device])
                        .set(stats.queue_length() as f64);
                    BYTES
                        .with_label_values(&[&*device, "read"])
                        .set(stats.total_bytes_read() as f64);
                    DURATION
                        .with_label_values(&[&*device, "read"])
                        .set(stats.total_duration_read());
                    OPS.with_label_values(&[&*device, "read"])
                        .set(stats.total_transfers_read() as f64);
                    BYTES
                        .with_label_values(&[&*device, "write"])
                        .set(stats.total_bytes_write() as f64);
                    DURATION
                        .with_label_values(&[&*device, "write"])
                        .set(stats.total_duration_write());
                    OPS.with_label_values(&[&*device, "write"])
                        .set(stats.total_transfers_write() as f64);
                    BYTES
                        .with_label_values(&[&*device, "free"])
                        .set(stats.total_bytes_free() as f64);
                    DURATION
                        .with_label_values(&[&*device, "free"])
                        .set(stats.total_duration_free());
                    OPS.with_label_values(&[&*device, "free"])
                        .set(stats.total_transfers_free() as f64);
                    DURATION
                        .with_label_values(&[&*device, "other"])
                        .set(stats.total_duration_other());
                    OPS.with_label_values(&[&*device, "other"])
                        .set(stats.total_transfers_other() as f64);
                }
            }
        }
        let metric_families = prometheus::gather();
        let encoder = TextEncoder::new();
        let body = encoder.encode_to_string(&metric_families)?;
        Ok(body)
    };
    // Now convert the error type again.
    inner().map_err(AppError)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli: Cli = Cli::parse();

    // Setup logger with default level info so we can see the messages from
    // prometheus_exporter.
    Builder::from_env(Env::default().default_filter_or("info")).init();

    // Parse address used to bind exporter to.
    let ia: IpAddr = cli.addr.parse().unwrap_or_else(|e| {
        eprintln!("Cannot parse address: {e}");
        exit(2);
    });
    let sa = SocketAddr::new(ia, cli.port);

    let app = Router::new()
        .route("/metrics", get(metrics))
        // Annoyingly, with_state requires its argument to be `Send` even if
        // we're using a single-threaded runtime.  So we must use Arc instead of
        // Rc.
        .with_state(Arc::new(cli));

    let listener = TcpListener::bind(sa).await.unwrap();
    axum::serve(listener, app).await.unwrap()
}
