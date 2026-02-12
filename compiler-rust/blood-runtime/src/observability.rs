//! Observability Infrastructure
//!
//! This module provides comprehensive observability for the Blood runtime:
//!
//! - **Metrics**: Counters, gauges, histograms for quantitative measurements
//! - **Tracing**: Spans, events, and distributed tracing support
//! - **Introspection**: Runtime state inspection APIs
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                     OBSERVABILITY                                │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                  │
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐          │
//! │  │   Metrics    │  │   Tracing    │  │ Introspection│          │
//! │  │  (counters,  │  │  (spans,     │  │   (state,    │          │
//! │  │   gauges)    │  │   events)    │  │   health)    │          │
//! │  └──────────────┘  └──────────────┘  └──────────────┘          │
//! │         │                 │                 │                   │
//! │         └─────────────────┼─────────────────┘                   │
//! │                           │                                     │
//! │              ┌────────────┴────────────┐                       │
//! │              │     MetricsRegistry     │                       │
//! │              │    TraceCollector       │                       │
//! │              │    StateInspector       │                       │
//! │              └─────────────────────────┘                       │
//! │                                                                  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! use blood_runtime::observability::{metrics, tracing, introspection};
//!
//! // Create a counter
//! let requests = metrics::counter("http_requests_total")
//!     .with_label("method", "GET")
//!     .register();
//!
//! // Increment the counter
//! requests.inc();
//!
//! // Create a span
//! let span = tracing::span("handle_request")
//!     .with_attribute("path", "/api/users")
//!     .start();
//!
//! // Do work...
//!
//! span.end();
//!
//! // Inspect runtime state
//! let stats = introspection::runtime_stats();
//! println!("Active fibers: {}", stats.active_fibers);
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::fiber_local::TraceContext;

// ============================================================================
// METRICS
// ============================================================================

/// Metric type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricType {
    /// A monotonically increasing counter.
    Counter,
    /// A value that can go up and down.
    Gauge,
    /// A distribution of values.
    Histogram,
}

/// A metric label (key-value pair).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Label {
    /// Label name.
    pub name: String,
    /// Label value.
    pub value: String,
}

impl Label {
    /// Create a new label.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// A monotonically increasing counter.
///
/// Counters are used to track things like request counts, error counts, etc.
#[derive(Debug)]
pub struct Counter {
    /// Counter name.
    name: String,
    /// Counter description.
    description: String,
    /// Labels for this counter instance.
    labels: Vec<Label>,
    /// Current value.
    value: AtomicU64,
}

impl Counter {
    /// Create a new counter.
    fn new(name: String, description: String, labels: Vec<Label>) -> Self {
        Self {
            name,
            description,
            labels,
            value: AtomicU64::new(0),
        }
    }

    /// Get the counter name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the counter description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Get the counter labels.
    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// Get the current counter value.
    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    /// Increment the counter by 1.
    pub fn inc(&self) {
        self.add(1);
    }

    /// Add a value to the counter.
    pub fn add(&self, n: u64) {
        self.value.fetch_add(n, Ordering::Relaxed);
    }
}

/// A gauge that can go up and down.
///
/// Gauges are used to track things like current memory usage, active connections, etc.
#[derive(Debug)]
pub struct Gauge {
    /// Gauge name.
    name: String,
    /// Gauge description.
    description: String,
    /// Labels for this gauge instance.
    labels: Vec<Label>,
    /// Current value (stored as i64 to support negative values).
    value: AtomicI64,
}

impl Gauge {
    /// Create a new gauge.
    fn new(name: String, description: String, labels: Vec<Label>) -> Self {
        Self {
            name,
            description,
            labels,
            value: AtomicI64::new(0),
        }
    }

    /// Get the gauge name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the gauge description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Get the gauge labels.
    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// Get the current gauge value.
    pub fn get(&self) -> i64 {
        self.value.load(Ordering::Relaxed)
    }

    /// Set the gauge to a specific value.
    pub fn set(&self, value: i64) {
        self.value.store(value, Ordering::Relaxed);
    }

    /// Increment the gauge by 1.
    pub fn inc(&self) {
        self.add(1);
    }

    /// Decrement the gauge by 1.
    pub fn dec(&self) {
        self.add(-1);
    }

    /// Add a value to the gauge.
    pub fn add(&self, n: i64) {
        self.value.fetch_add(n, Ordering::Relaxed);
    }
}

/// A histogram for measuring distributions.
///
/// Histograms are used to track things like request latencies, response sizes, etc.
#[derive(Debug)]
pub struct Histogram {
    /// Histogram name.
    name: String,
    /// Histogram description.
    description: String,
    /// Labels for this histogram instance.
    labels: Vec<Label>,
    /// Bucket boundaries.
    buckets: Vec<f64>,
    /// Count per bucket.
    bucket_counts: Vec<AtomicU64>,
    /// Total count.
    count: AtomicU64,
    /// Sum of all observations.
    sum: Mutex<f64>,
}

impl Histogram {
    /// Default bucket boundaries for latency histograms (in seconds).
    pub const DEFAULT_BUCKETS: [f64; 11] = [
        0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
    ];

    /// Create a new histogram with default buckets.
    fn new(name: String, description: String, labels: Vec<Label>) -> Self {
        Self::with_buckets(name, description, labels, Self::DEFAULT_BUCKETS.to_vec())
    }

    /// Create a new histogram with custom buckets.
    fn with_buckets(
        name: String,
        description: String,
        labels: Vec<Label>,
        buckets: Vec<f64>,
    ) -> Self {
        let bucket_counts = buckets.iter().map(|_| AtomicU64::new(0)).collect();
        Self {
            name,
            description,
            labels,
            buckets,
            bucket_counts,
            count: AtomicU64::new(0),
            sum: Mutex::new(0.0),
        }
    }

    /// Get the histogram name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the histogram description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Get the histogram labels.
    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// Observe a value.
    pub fn observe(&self, value: f64) {
        // Increment count
        self.count.fetch_add(1, Ordering::Relaxed);

        // Add to sum
        if let Ok(mut sum) = self.sum.lock() {
            *sum += value;
        }

        // Find the right bucket
        for (i, boundary) in self.buckets.iter().enumerate() {
            if value <= *boundary {
                self.bucket_counts[i].fetch_add(1, Ordering::Relaxed);
                break;
            }
        }
    }

    /// Get the total count of observations.
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Get the sum of all observations.
    pub fn sum(&self) -> f64 {
        *self.sum.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Get the bucket boundaries.
    pub fn buckets(&self) -> &[f64] {
        &self.buckets
    }

    /// Get the bucket counts.
    pub fn bucket_counts(&self) -> Vec<u64> {
        self.bucket_counts
            .iter()
            .map(|c| c.load(Ordering::Relaxed))
            .collect()
    }

    /// Time a closure and observe its duration.
    pub fn time<F, T>(&self, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        let start = Instant::now();
        let result = f();
        self.observe(start.elapsed().as_secs_f64());
        result
    }
}

/// Builder for creating counters.
pub struct CounterBuilder {
    name: String,
    description: String,
    labels: Vec<Label>,
}

impl CounterBuilder {
    /// Create a new counter builder.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            labels: Vec::new(),
        }
    }

    /// Set the counter description.
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Add a label to the counter.
    pub fn with_label(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.push(Label::new(name, value));
        self
    }

    /// Build and register the counter.
    pub fn register(self) -> Arc<Counter> {
        let counter = Arc::new(Counter::new(self.name, self.description, self.labels));
        METRICS_REGISTRY.register_counter(counter.clone());
        counter
    }
}

/// Builder for creating gauges.
pub struct GaugeBuilder {
    name: String,
    description: String,
    labels: Vec<Label>,
}

impl GaugeBuilder {
    /// Create a new gauge builder.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            labels: Vec::new(),
        }
    }

    /// Set the gauge description.
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Add a label to the gauge.
    pub fn with_label(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.push(Label::new(name, value));
        self
    }

    /// Build and register the gauge.
    pub fn register(self) -> Arc<Gauge> {
        let gauge = Arc::new(Gauge::new(self.name, self.description, self.labels));
        METRICS_REGISTRY.register_gauge(gauge.clone());
        gauge
    }
}

/// Builder for creating histograms.
pub struct HistogramBuilder {
    name: String,
    description: String,
    labels: Vec<Label>,
    buckets: Option<Vec<f64>>,
}

impl HistogramBuilder {
    /// Create a new histogram builder.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            labels: Vec::new(),
            buckets: None,
        }
    }

    /// Set the histogram description.
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Add a label to the histogram.
    pub fn with_label(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.push(Label::new(name, value));
        self
    }

    /// Set custom bucket boundaries.
    pub fn buckets(mut self, buckets: Vec<f64>) -> Self {
        self.buckets = Some(buckets);
        self
    }

    /// Build and register the histogram.
    pub fn register(self) -> Arc<Histogram> {
        let histogram = Arc::new(match self.buckets {
            Some(buckets) => Histogram::with_buckets(self.name, self.description, self.labels, buckets),
            None => Histogram::new(self.name, self.description, self.labels),
        });
        METRICS_REGISTRY.register_histogram(histogram.clone());
        histogram
    }
}

/// Global metrics registry.
struct MetricsRegistry {
    counters: RwLock<HashMap<String, Arc<Counter>>>,
    gauges: RwLock<HashMap<String, Arc<Gauge>>>,
    histograms: RwLock<HashMap<String, Arc<Histogram>>>,
}

impl MetricsRegistry {
    fn new() -> Self {
        Self {
            counters: RwLock::new(HashMap::new()),
            gauges: RwLock::new(HashMap::new()),
            histograms: RwLock::new(HashMap::new()),
        }
    }

    fn register_counter(&self, counter: Arc<Counter>) {
        if let Ok(mut counters) = self.counters.write() {
            counters.insert(counter.name.clone(), counter);
        }
    }

    fn register_gauge(&self, gauge: Arc<Gauge>) {
        if let Ok(mut gauges) = self.gauges.write() {
            gauges.insert(gauge.name.clone(), gauge);
        }
    }

    fn register_histogram(&self, histogram: Arc<Histogram>) {
        if let Ok(mut histograms) = self.histograms.write() {
            histograms.insert(histogram.name.clone(), histogram);
        }
    }

    fn get_counter(&self, name: &str) -> Option<Arc<Counter>> {
        self.counters.read().ok()?.get(name).cloned()
    }

    fn get_gauge(&self, name: &str) -> Option<Arc<Gauge>> {
        self.gauges.read().ok()?.get(name).cloned()
    }

    fn get_histogram(&self, name: &str) -> Option<Arc<Histogram>> {
        self.histograms.read().ok()?.get(name).cloned()
    }

    fn all_counters(&self) -> Vec<Arc<Counter>> {
        self.counters
            .read()
            .map(|c| c.values().cloned().collect())
            .unwrap_or_default()
    }

    fn all_gauges(&self) -> Vec<Arc<Gauge>> {
        self.gauges
            .read()
            .map(|g| g.values().cloned().collect())
            .unwrap_or_default()
    }

    fn all_histograms(&self) -> Vec<Arc<Histogram>> {
        self.histograms
            .read()
            .map(|h| h.values().cloned().collect())
            .unwrap_or_default()
    }
}

static METRICS_REGISTRY: std::sync::LazyLock<MetricsRegistry> =
    std::sync::LazyLock::new(MetricsRegistry::new);

/// Metrics module public API.
pub mod metrics {
    use super::*;

    /// Create a counter builder.
    pub fn counter(name: impl Into<String>) -> CounterBuilder {
        CounterBuilder::new(name)
    }

    /// Create a gauge builder.
    pub fn gauge(name: impl Into<String>) -> GaugeBuilder {
        GaugeBuilder::new(name)
    }

    /// Create a histogram builder.
    pub fn histogram(name: impl Into<String>) -> HistogramBuilder {
        HistogramBuilder::new(name)
    }

    /// Get a registered counter by name.
    pub fn get_counter(name: &str) -> Option<Arc<Counter>> {
        METRICS_REGISTRY.get_counter(name)
    }

    /// Get a registered gauge by name.
    pub fn get_gauge(name: &str) -> Option<Arc<Gauge>> {
        METRICS_REGISTRY.get_gauge(name)
    }

    /// Get a registered histogram by name.
    pub fn get_histogram(name: &str) -> Option<Arc<Histogram>> {
        METRICS_REGISTRY.get_histogram(name)
    }

    /// Get all registered counters.
    pub fn all_counters() -> Vec<Arc<Counter>> {
        METRICS_REGISTRY.all_counters()
    }

    /// Get all registered gauges.
    pub fn all_gauges() -> Vec<Arc<Gauge>> {
        METRICS_REGISTRY.all_gauges()
    }

    /// Get all registered histograms.
    pub fn all_histograms() -> Vec<Arc<Histogram>> {
        METRICS_REGISTRY.all_histograms()
    }
}

// ============================================================================
// TRACING
// ============================================================================

/// Span status indicating the result of an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpanStatus {
    /// Status not set.
    #[default]
    Unset,
    /// Operation completed successfully.
    Ok,
    /// Operation failed with an error.
    Error,
}

/// An attribute value for spans and events.
#[derive(Debug, Clone)]
pub enum AttributeValue {
    /// String value.
    String(String),
    /// Boolean value.
    Bool(bool),
    /// Integer value.
    Int(i64),
    /// Float value.
    Float(f64),
}

impl From<&str> for AttributeValue {
    fn from(s: &str) -> Self {
        AttributeValue::String(s.to_string())
    }
}

impl From<String> for AttributeValue {
    fn from(s: String) -> Self {
        AttributeValue::String(s)
    }
}

impl From<bool> for AttributeValue {
    fn from(b: bool) -> Self {
        AttributeValue::Bool(b)
    }
}

impl From<i64> for AttributeValue {
    fn from(i: i64) -> Self {
        AttributeValue::Int(i)
    }
}

impl From<f64> for AttributeValue {
    fn from(f: f64) -> Self {
        AttributeValue::Float(f)
    }
}

/// A span represents a single operation within a trace.
#[derive(Debug)]
pub struct Span {
    /// Span ID.
    id: u64,
    /// Trace ID.
    trace_id: u128,
    /// Parent span ID (if any).
    parent_id: Option<u64>,
    /// Span name.
    name: String,
    /// Start time.
    start_time: Instant,
    /// End time (set when span ends).
    end_time: Mutex<Option<Instant>>,
    /// Span attributes.
    attributes: RwLock<HashMap<String, AttributeValue>>,
    /// Span events.
    events: RwLock<Vec<SpanEvent>>,
    /// Span status.
    status: Mutex<SpanStatus>,
    /// Status message (for errors).
    status_message: Mutex<Option<String>>,
}

impl Span {
    /// Create a new span.
    fn new(name: String, trace_id: u128, parent_id: Option<u64>) -> Self {
        static SPAN_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

        Self {
            id: SPAN_ID_COUNTER.fetch_add(1, Ordering::SeqCst),
            trace_id,
            parent_id,
            name,
            start_time: Instant::now(),
            end_time: Mutex::new(None),
            attributes: RwLock::new(HashMap::new()),
            events: RwLock::new(Vec::new()),
            status: Mutex::new(SpanStatus::Unset),
            status_message: Mutex::new(None),
        }
    }

    /// Get the span ID.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Get the trace ID.
    pub fn trace_id(&self) -> u128 {
        self.trace_id
    }

    /// Get the parent span ID.
    pub fn parent_id(&self) -> Option<u64> {
        self.parent_id
    }

    /// Get the span name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the span start time.
    pub fn start_time(&self) -> Instant {
        self.start_time
    }

    /// Get the span duration.
    pub fn duration(&self) -> Duration {
        let end = self
            .end_time
            .lock()
            .ok()
            .and_then(|e| *e)
            .unwrap_or_else(Instant::now);
        end - self.start_time
    }

    /// Set an attribute on the span.
    pub fn set_attribute(&self, key: impl Into<String>, value: impl Into<AttributeValue>) {
        if let Ok(mut attrs) = self.attributes.write() {
            attrs.insert(key.into(), value.into());
        }
    }

    /// Add an event to the span.
    pub fn add_event(&self, name: impl Into<String>) {
        self.add_event_with_attributes(name, HashMap::new());
    }

    /// Add an event with attributes to the span.
    pub fn add_event_with_attributes(
        &self,
        name: impl Into<String>,
        attributes: HashMap<String, AttributeValue>,
    ) {
        if let Ok(mut events) = self.events.write() {
            events.push(SpanEvent {
                name: name.into(),
                timestamp: Instant::now(),
                attributes,
            });
        }
    }

    /// Set the span status.
    pub fn set_status(&self, status: SpanStatus) {
        if let Ok(mut s) = self.status.lock() {
            *s = status;
        }
    }

    /// Set the span status with a message.
    pub fn set_error(&self, message: impl Into<String>) {
        self.set_status(SpanStatus::Error);
        if let Ok(mut msg) = self.status_message.lock() {
            *msg = Some(message.into());
        }
    }

    /// End the span.
    pub fn end(&self) {
        if let Ok(mut end) = self.end_time.lock() {
            if end.is_none() {
                *end = Some(Instant::now());
            }
        }

        // Record the span in the collector
        TRACE_COLLECTOR.record_span(self);
    }

    /// Check if the span has ended.
    pub fn is_ended(&self) -> bool {
        self.end_time
            .lock()
            .ok()
            .map(|e| e.is_some())
            .unwrap_or(false)
    }

    /// Create a trace context from this span.
    pub fn to_trace_context(&self) -> TraceContext {
        TraceContext {
            trace_id: self.trace_id,
            span_id: self.id,
            parent_span_id: self.parent_id,
            sampled: true,
        }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        // Ensure span is ended when dropped
        if !self.is_ended() {
            self.end();
        }
    }
}

/// An event within a span.
#[derive(Debug, Clone)]
pub struct SpanEvent {
    /// Event name.
    pub name: String,
    /// Event timestamp.
    pub timestamp: Instant,
    /// Event attributes.
    pub attributes: HashMap<String, AttributeValue>,
}

/// Builder for creating spans.
pub struct SpanBuilder {
    name: String,
    parent_id: Option<u64>,
    trace_id: Option<u128>,
    attributes: HashMap<String, AttributeValue>,
}

impl SpanBuilder {
    /// Create a new span builder.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            parent_id: None,
            trace_id: None,
            attributes: HashMap::new(),
        }
    }

    /// Set the parent span.
    pub fn parent(mut self, parent: &Span) -> Self {
        self.parent_id = Some(parent.id());
        self.trace_id = Some(parent.trace_id());
        self
    }

    /// Set the trace context.
    pub fn trace_context(mut self, ctx: &TraceContext) -> Self {
        self.trace_id = Some(ctx.trace_id);
        self.parent_id = ctx.parent_span_id;
        self
    }

    /// Add an attribute to the span.
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<AttributeValue>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// Start the span.
    pub fn start(self) -> Arc<Span> {
        let trace_id = self.trace_id.unwrap_or_else(|| {
            // Generate a new trace ID
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            now
        });

        let span = Arc::new(Span::new(self.name, trace_id, self.parent_id));

        // Set initial attributes
        for (key, value) in self.attributes {
            span.set_attribute(key, value);
        }

        span
    }
}

/// Trace collector for recording completed spans.
struct TraceCollector {
    /// Completed spans.
    spans: RwLock<Vec<CompletedSpan>>,
    /// Maximum number of spans to keep.
    max_spans: usize,
    /// Export handlers.
    exporters: RwLock<Vec<Box<dyn SpanExporter + Send + Sync>>>,
}

/// A completed span record.
#[derive(Debug, Clone)]
pub struct CompletedSpan {
    /// Span ID.
    pub id: u64,
    /// Trace ID.
    pub trace_id: u128,
    /// Parent span ID.
    pub parent_id: Option<u64>,
    /// Span name.
    pub name: String,
    /// Start timestamp (unix nanos).
    pub start_time_unix_nano: u128,
    /// End timestamp (unix nanos).
    pub end_time_unix_nano: u128,
    /// Duration in nanoseconds.
    pub duration_nanos: u64,
    /// Attributes.
    pub attributes: HashMap<String, AttributeValue>,
    /// Events.
    pub events: Vec<SpanEvent>,
    /// Status.
    pub status: SpanStatus,
    /// Status message.
    pub status_message: Option<String>,
}

impl TraceCollector {
    fn new() -> Self {
        Self {
            spans: RwLock::new(Vec::new()),
            max_spans: 10000,
            exporters: RwLock::new(Vec::new()),
        }
    }

    fn record_span(&self, span: &Span) {
        let completed = CompletedSpan {
            id: span.id,
            trace_id: span.trace_id,
            parent_id: span.parent_id,
            name: span.name.clone(),
            start_time_unix_nano: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                - span.duration().as_nanos(),
            end_time_unix_nano: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            duration_nanos: span.duration().as_nanos() as u64,
            attributes: span
                .attributes
                .read()
                .map(|a| a.clone())
                .unwrap_or_default(),
            events: span.events.read().map(|e| e.clone()).unwrap_or_default(),
            status: span.status.lock().map(|s| *s).unwrap_or_default(),
            status_message: span.status_message.lock().ok().and_then(|m| m.clone()),
        };

        // Export to registered exporters
        if let Ok(exporters) = self.exporters.read() {
            for exporter in exporters.iter() {
                exporter.export(&completed);
            }
        }

        // Store in local buffer
        if let Ok(mut spans) = self.spans.write() {
            spans.push(completed);
            if spans.len() > self.max_spans {
                spans.remove(0);
            }
        }
    }

    fn add_exporter(&self, exporter: Box<dyn SpanExporter + Send + Sync>) {
        if let Ok(mut exporters) = self.exporters.write() {
            exporters.push(exporter);
        }
    }

    fn recent_spans(&self, limit: usize) -> Vec<CompletedSpan> {
        self.spans
            .read()
            .map(|spans| {
                let start = if spans.len() > limit {
                    spans.len() - limit
                } else {
                    0
                };
                spans[start..].to_vec()
            })
            .unwrap_or_default()
    }

    fn spans_for_trace(&self, trace_id: u128) -> Vec<CompletedSpan> {
        self.spans
            .read()
            .map(|spans| {
                spans
                    .iter()
                    .filter(|s| s.trace_id == trace_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

static TRACE_COLLECTOR: std::sync::LazyLock<TraceCollector> =
    std::sync::LazyLock::new(TraceCollector::new);

/// Trait for exporting spans to external systems.
pub trait SpanExporter {
    /// Export a completed span.
    fn export(&self, span: &CompletedSpan);
}

/// Tracing module public API.
pub mod tracing {
    use super::*;

    /// Create a span builder.
    pub fn span(name: impl Into<String>) -> SpanBuilder {
        SpanBuilder::new(name)
    }

    /// Add a span exporter.
    pub fn add_exporter(exporter: Box<dyn SpanExporter + Send + Sync>) {
        TRACE_COLLECTOR.add_exporter(exporter);
    }

    /// Get recent spans.
    pub fn recent_spans(limit: usize) -> Vec<CompletedSpan> {
        TRACE_COLLECTOR.recent_spans(limit)
    }

    /// Get all spans for a trace.
    pub fn spans_for_trace(trace_id: u128) -> Vec<CompletedSpan> {
        TRACE_COLLECTOR.spans_for_trace(trace_id)
    }
}

// ============================================================================
// INTROSPECTION
// ============================================================================

/// Runtime statistics.
#[derive(Debug, Clone)]
pub struct RuntimeStats {
    /// Number of active fibers.
    pub active_fibers: usize,
    /// Number of pending fibers.
    pub pending_fibers: usize,
    /// Number of worker threads.
    pub worker_count: usize,
    /// Total memory allocated.
    pub total_memory_bytes: usize,
    /// Number of active regions.
    pub active_regions: usize,
    /// Number of active cancellation tokens.
    pub active_cancellation_tokens: usize,
    /// Uptime in seconds.
    pub uptime_seconds: f64,
}

/// Memory statistics.
#[derive(Debug, Clone)]
pub struct MemoryStats {
    /// Total allocated bytes.
    pub allocated_bytes: usize,
    /// Total freed bytes.
    pub freed_bytes: usize,
    /// Current live bytes.
    pub live_bytes: usize,
    /// Number of allocations.
    pub allocation_count: usize,
    /// Number of deallocations.
    pub deallocation_count: usize,
    /// Number of regions.
    pub region_count: usize,
    /// Number of persistent slots.
    pub persistent_slot_count: usize,
}

/// Scheduler statistics.
#[derive(Debug, Clone)]
pub struct SchedulerStats {
    /// Total fibers spawned.
    pub fibers_spawned: u64,
    /// Total fibers completed.
    pub fibers_completed: u64,
    /// Total fibers cancelled.
    pub fibers_cancelled: u64,
    /// Currently running fibers.
    pub fibers_running: usize,
    /// Currently suspended fibers.
    pub fibers_suspended: usize,
    /// Total context switches.
    pub context_switches: u64,
    /// Total work stolen.
    pub work_stolen: u64,
}

/// Health check result.
#[derive(Debug, Clone)]
pub struct HealthStatus {
    /// Overall health status.
    pub healthy: bool,
    /// Individual check results.
    pub checks: Vec<HealthCheck>,
}

/// Individual health check.
#[derive(Debug, Clone)]
pub struct HealthCheck {
    /// Check name.
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Optional message.
    pub message: Option<String>,
}

/// Runtime start time.
static RUNTIME_START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);

/// Introspection module public API.
pub mod introspection {
    use super::*;

    /// Get runtime statistics.
    pub fn runtime_stats() -> RuntimeStats {
        // Initialize start time if not already done
        let _ = *RUNTIME_START;

        RuntimeStats {
            active_fibers: 0, // Would query scheduler
            pending_fibers: 0,
            worker_count: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            total_memory_bytes: crate::memory::total_allocated(),
            active_regions: crate::memory::active_region_count(),
            active_cancellation_tokens: crate::cancellation::active_token_count(),
            uptime_seconds: RUNTIME_START.elapsed().as_secs_f64(),
        }
    }

    /// Get memory statistics.
    pub fn memory_stats() -> MemoryStats {
        let stats = crate::memory::allocation_stats();
        MemoryStats {
            allocated_bytes: stats.total_allocated,
            freed_bytes: stats.total_freed,
            live_bytes: stats.current_live,
            allocation_count: stats.allocation_count,
            deallocation_count: stats.deallocation_count,
            region_count: crate::memory::active_region_count(),
            persistent_slot_count: crate::memory::persistent_slot_count(),
        }
    }

    /// Perform a health check.
    pub fn health_check() -> HealthStatus {
        let mut checks = Vec::new();

        // Memory check
        let mem_stats = memory_stats();
        let memory_ok = mem_stats.live_bytes < 1024 * 1024 * 1024; // Less than 1GB
        checks.push(HealthCheck {
            name: "memory".to_string(),
            passed: memory_ok,
            message: if memory_ok {
                None
            } else {
                Some(format!("High memory usage: {} bytes", mem_stats.live_bytes))
            },
        });

        // Cancellation tokens check
        let token_count = crate::cancellation::active_token_count();
        let tokens_ok = token_count < 10000;
        checks.push(HealthCheck {
            name: "cancellation_tokens".to_string(),
            passed: tokens_ok,
            message: if tokens_ok {
                None
            } else {
                Some(format!("High cancellation token count: {}", token_count))
            },
        });

        let healthy = checks.iter().all(|c| c.passed);
        HealthStatus { healthy, checks }
    }

    /// Get uptime in seconds.
    pub fn uptime() -> Duration {
        RUNTIME_START.elapsed()
    }

    /// Get the number of available CPUs.
    pub fn cpu_count() -> usize {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod metrics_tests {
        use super::*;

        #[test]
        fn test_counter() {
            let counter = CounterBuilder::new("test_counter")
                .description("A test counter")
                .with_label("env", "test")
                .register();

            assert_eq!(counter.get(), 0);
            counter.inc();
            assert_eq!(counter.get(), 1);
            counter.add(5);
            assert_eq!(counter.get(), 6);
        }

        #[test]
        fn test_gauge() {
            let gauge = GaugeBuilder::new("test_gauge")
                .description("A test gauge")
                .register();

            assert_eq!(gauge.get(), 0);
            gauge.set(100);
            assert_eq!(gauge.get(), 100);
            gauge.inc();
            assert_eq!(gauge.get(), 101);
            gauge.dec();
            assert_eq!(gauge.get(), 100);
            gauge.add(-50);
            assert_eq!(gauge.get(), 50);
        }

        #[test]
        fn test_histogram() {
            let histogram = HistogramBuilder::new("test_histogram")
                .description("A test histogram")
                .register();

            assert_eq!(histogram.count(), 0);
            histogram.observe(0.1);
            histogram.observe(0.5);
            histogram.observe(1.0);
            assert_eq!(histogram.count(), 3);
            assert!((histogram.sum() - 1.6).abs() < 0.001);
        }

        #[test]
        fn test_histogram_time() {
            let histogram = HistogramBuilder::new("test_timer")
                .buckets(vec![0.001, 0.01, 0.1, 1.0])
                .register();

            let result = histogram.time(|| {
                std::thread::sleep(Duration::from_millis(1));
                42
            });

            assert_eq!(result, 42);
            assert_eq!(histogram.count(), 1);
            assert!(histogram.sum() > 0.0);
        }
    }

    mod tracing_tests {
        use super::*;

        #[test]
        fn test_span_creation() {
            let span = tracing::span("test_operation").start();
            assert!(!span.name().is_empty());
            assert!(span.trace_id() > 0);
            assert!(span.id() > 0);
        }

        #[test]
        fn test_span_attributes() {
            let span = tracing::span("test_with_attrs")
                .with_attribute("key1", "value1")
                .start();

            span.set_attribute("key2", 42i64);
            span.end();
        }

        #[test]
        fn test_span_events() {
            let span = tracing::span("test_with_events").start();
            span.add_event("event1");
            span.add_event_with_attributes("event2", {
                let mut attrs = HashMap::new();
                attrs.insert("attr1".to_string(), AttributeValue::String("value".to_string()));
                attrs
            });
            span.end();
        }

        #[test]
        fn test_span_status() {
            let span = tracing::span("test_status").start();
            span.set_status(SpanStatus::Ok);
            span.end();

            let span2 = tracing::span("test_error").start();
            span2.set_error("Something went wrong");
            span2.end();
        }

        #[test]
        fn test_parent_span() {
            let parent = tracing::span("parent").start();
            let child = tracing::span("child").parent(&parent).start();

            assert_eq!(child.trace_id(), parent.trace_id());
            assert_eq!(child.parent_id(), Some(parent.id()));

            child.end();
            parent.end();
        }
    }

    mod introspection_tests {
        use super::*;

        #[test]
        fn test_runtime_stats() {
            let stats = introspection::runtime_stats();
            assert!(stats.worker_count >= 1);
            assert!(stats.uptime_seconds >= 0.0);
        }

        #[test]
        fn test_memory_stats() {
            let stats = introspection::memory_stats();
            assert!(stats.allocation_count >= stats.deallocation_count);
        }

        #[test]
        fn test_health_check() {
            let health = introspection::health_check();
            assert!(!health.checks.is_empty());
        }

        #[test]
        fn test_uptime() {
            let uptime = introspection::uptime();
            assert!(uptime >= Duration::ZERO);
        }

        #[test]
        fn test_cpu_count() {
            let count = introspection::cpu_count();
            assert!(count >= 1);
        }
    }
}
