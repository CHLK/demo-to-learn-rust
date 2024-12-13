use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    runtime,
    trace::{self, RandomIdGenerator},
    Resource,
};

use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    Layer,
};

#[allow(dead_code)]
fn init_tracing() -> anyhow::Result<()> {
    // console layer for tracing-subscriber
    let console = fmt::Layer::new()
        .with_span_events(FmtSpan::CLOSE)
        .pretty()
        .with_filter(LevelFilter::DEBUG);

    // file appender layer for tracing-subscriber
    let file_appender = tracing_appender::rolling::daily("/tmp/logs", "ecosystem.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let file = fmt::Layer::new()
        .with_writer(non_blocking)
        .pretty()
        .with_filter(LevelFilter::INFO);

    // opentelemetry tracing layer for tracing-subscriber
    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint("http://localhost:4317"),
        )
        .with_trace_config(
            trace::config()
                .with_id_generator(RandomIdGenerator::default())
                .with_max_events_per_span(32)
                .with_max_attributes_per_span(64)
                .with_resource(Resource::new(vec![KeyValue::new(
                    "service.name",
                    "axum-tracing",
                )])),
        )
        .install_batch(runtime::Tokio)?;
    let opentelemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    tracing_subscriber::registry()
        .with(console)
        .with(file)
        .with(opentelemetry)
        .init();

    Ok(())
}
