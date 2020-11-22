mod bytes_processed;
mod errors;
mod events_processed;
mod host;
mod uptime;

use super::components::{self, Component, COMPONENTS};
use crate::{
    event::{Event, Metric, MetricValue},
    metrics::{capture_metrics, get_controller, Controller},
};
use async_graphql::{validators::IntRange, Interface, Object, Subscription};
use async_stream::stream;
use chrono::{DateTime, Utc};
use itertools::Itertools;
use lazy_static::lazy_static;
use std::{collections::BTreeMap, sync::Arc};
use tokio::{
    stream::{Stream, StreamExt},
    time::Duration,
};

pub use bytes_processed::{
    BytesProcessedTotal, ComponentBytesProcessedThroughput, ComponentBytesProcessedTotal,
};
pub use errors::{ComponentErrorsTotal, ErrorsTotal};
pub use events_processed::{
    ComponentEventsProcessedThroughput, ComponentEventsProcessedTotal, EventsProcessedTotal,
};
pub use host::HostMetrics;
pub use uptime::Uptime;

lazy_static! {
    static ref GLOBAL_CONTROLLER: Arc<&'static Controller> =
        Arc::new(get_controller().expect("Metrics system not initialized. Please report."));
}

#[derive(Interface)]
#[graphql(field(name = "timestamp", type = "Option<DateTime<Utc>>"))]
pub enum MetricType {
    Uptime(Uptime),
    EventsProcessedTotal(EventsProcessedTotal),
    BytesProcessedTotal(BytesProcessedTotal),
}

#[derive(Default)]
pub struct MetricsQuery;

#[Object]
impl MetricsQuery {
    /// Vector host metrics
    async fn host_metrics(&self) -> HostMetrics {
        HostMetrics::new()
    }
}

#[derive(Default)]
pub struct MetricsSubscription;

#[Subscription]
impl MetricsSubscription {
    /// Metrics for how long the Vector instance has been running.
    async fn uptime(
        &self,
        #[graphql(default = 1000, validator(IntRange(min = "10", max = "60_000")))] interval: i32,
    ) -> impl Stream<Item = Uptime> {
        get_metrics(interval).filter_map(|m| match m.name.as_str() {
            "uptime_seconds" => Some(Uptime::new(m)),
            _ => None,
        })
    }

    /// Events processed metrics.
    async fn events_processed_total(
        &self,
        #[graphql(default = 1000, validator(IntRange(min = "10", max = "60_000")))] interval: i32,
    ) -> impl Stream<Item = EventsProcessedTotal> {
        get_metrics(interval).filter_map(|m| match m.name.as_str() {
            "events_processed_total" => Some(EventsProcessedTotal::new(m)),
            _ => None,
        })
    }

    /// Events processed throughput, sampled over a provided millisecond `interval`.
    async fn events_processed_throughput(
        &self,
        #[graphql(default = 1000, validator(IntRange(min = "10", max = "60_000")))] interval: i32,
    ) -> impl Stream<Item = i64> {
        counter_throughput(interval, &|m| m.name == "events_processed_total")
            .map(|(_, throughput)| throughput as i64)
    }

    /// Component events processed throughputs over `interval`.
    async fn component_events_processed_throughputs(
        &self,
        #[graphql(default = 1000, validator(IntRange(min = "10", max = "60_000")))] interval: i32,
    ) -> impl Stream<Item = Vec<ComponentEventsProcessedThroughput>> {
        component_counter_throughputs(interval, &|m| m.name == "events_processed_total").map(|m| {
            m.into_iter()
                .map(|(m, throughput)| {
                    ComponentEventsProcessedThroughput::new(
                        m.tag_value("component_name").unwrap(),
                        throughput as i64,
                    )
                })
                .collect()
        })
    }

    /// Component events processed metrics over `interval`.
    async fn component_events_processed_totals(
        &self,
        #[graphql(default = 1000, validator(IntRange(min = "10", max = "60_000")))] interval: i32,
    ) -> impl Stream<Item = Vec<ComponentEventsProcessedTotal>> {
        component_counter_metrics(interval, &|m| m.name == "events_processed_total").map(|m| {
            m.into_iter()
                .map(ComponentEventsProcessedTotal::new)
                .collect()
        })
    }

    /// Bytes processed metrics.
    async fn bytes_processed_total(
        &self,
        #[graphql(default = 1000, validator(IntRange(min = "10", max = "60_000")))] interval: i32,
    ) -> impl Stream<Item = BytesProcessedTotal> {
        get_metrics(interval).filter_map(|m| match m.name.as_str() {
            "processed_bytes_total" => Some(BytesProcessedTotal::new(m)),
            _ => None,
        })
    }

    /// Bytes processed throughput, sampled over a provided millisecond `interval`.
    async fn bytes_processed_throughput(
        &self,
        #[graphql(default = 1000, validator(IntRange(min = "10", max = "60_000")))] interval: i32,
    ) -> impl Stream<Item = i64> {
        counter_throughput(interval, &|m| m.name == "processed_bytes_total")
            .map(|(_, throughput)| throughput as i64)
    }

    /// Component bytes processed metrics, over `interval`.
    async fn component_bytes_processed_totals(
        &self,
        #[graphql(default = 1000, validator(IntRange(min = "10", max = "60_000")))] interval: i32,
    ) -> impl Stream<Item = Vec<ComponentBytesProcessedTotal>> {
        component_counter_metrics(interval, &|m| m.name == "processed_bytes_total").map(|m| {
            m.into_iter()
                .map(ComponentBytesProcessedTotal::new)
                .collect()
        })
    }

    /// Component bytes processed throughputs, over `interval`
    async fn component_bytes_processed_throughputs(
        &self,
        #[graphql(default = 1000, validator(IntRange(min = "10", max = "60_000")))] interval: i32,
    ) -> impl Stream<Item = Vec<ComponentBytesProcessedThroughput>> {
        component_counter_throughputs(interval, &|m| m.name == "processed_bytes_total").map(|m| {
            m.into_iter()
                .map(|(m, throughput)| {
                    ComponentBytesProcessedThroughput::new(
                        m.tag_value("component_name").unwrap(),
                        throughput as i64,
                    )
                })
                .collect()
        })
    }

    /// Total error metrics.
    async fn errors_total(
        &self,
        #[graphql(default = 1000, validator(IntRange(min = "10", max = "60_000")))] interval: i32,
    ) -> impl Stream<Item = ErrorsTotal> {
        get_metrics(interval)
            .filter(|m| m.name.ends_with("_errors_total"))
            .map(ErrorsTotal::new)
    }

    /// Component errors metrics, over `interval`.
    async fn component_errors_totals(
        &self,
        #[graphql(default = 1000, validator(IntRange(min = "10", max = "60_000")))] interval: i32,
    ) -> impl Stream<Item = Vec<ComponentErrorsTotal>> {
        component_counter_metrics(interval, &|m| m.name.ends_with("_errors_total"))
            .map(|m| m.into_iter().map(ComponentErrorsTotal::new).collect())
    }

    /// All metrics.
    async fn metrics(
        &self,
        #[graphql(default = 1000, validator(IntRange(min = "10", max = "60_000")))] interval: i32,
    ) -> impl Stream<Item = MetricType> {
        get_metrics(interval).filter_map(|m| match m.name.as_str() {
            "uptime_seconds" => Some(MetricType::Uptime(m.into())),
            "events_processed_total" => Some(MetricType::EventsProcessedTotal(m.into())),
            "processed_bytes_total" => Some(MetricType::BytesProcessedTotal(m.into())),
            _ => None,
        })
    }
}

/// Returns a stream of `Metric`s, collected at the provided millisecond interval.
fn get_metrics(interval: i32) -> impl Stream<Item = Metric> {
    let controller = get_controller().unwrap();
    let mut interval = tokio::time::interval(Duration::from_millis(interval as u64));

    stream! {
        loop {
            interval.tick().await;
            for ev in capture_metrics(&controller) {
                if let Event::Metric(m) = ev {
                    yield m;
                }
            }
        }
    }
}

/// Returns a stream of `Metrics`, sorted into source, transform and sinks, in that order,
/// where same metrics with different `origin` label under the same componenets are aggregated.
/// Metrics are collected into a `Vec<Metric>`, yielding at `inverval` milliseconds.
fn component_metrics(interval: i32) -> impl Stream<Item = Vec<Metric>> {
    let controller = get_controller().unwrap();
    let mut interval = tokio::time::interval(Duration::from_millis(interval as u64));

    stream! {
        loop {
            interval.tick().await;

            // Sort each interval of metrics by key
            let mut metrics_it=capture_metrics(&controller)
            .filter_map(|m| match m {
                Event::Metric(m) => match m.tag_value("component_name") {
                    Some(name) => match COMPONENTS.read().expect(components::INVARIANT).get(&name) {
                        Some(t) => Some(match t {
                            Component::Source(_) => (m, 1),
                            Component::Transform(_) => (m, 2),
                            Component::Sink(_) => (m, 3),
                        }),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            })
            .sorted_by_key(|m| (m.1,m.0.name.clone()))
            .map(|(m, _)| m);

            // Aggregate metrics per componenet
            let mut metrics=Vec::new();
            let mut component=Vec::new();
            let mut component_name=None;
            while let Some(metric)=metrics_it.next(){
                let name=metric.tag_value("component_name");
                if component_name != name{
                    aggregate(&mut component,&mut metrics);
                    component_name=name;
                }
                component.push((metric,false));
            }
            aggregate(&mut component,&mut metrics);

            yield metrics;
        }
    }
}

/// Same metrics with different `origin` label are aggregated.
/// `origin` label is removed in the process
fn aggregate(metrics: &mut Vec<(Metric, bool)>, out: &mut Vec<Metric>) {
    // Remove `origin` so that we can sort metrics.
    for (metric, priority) in metrics.iter_mut() {
        let origin = metric.tags.as_mut().and_then(|tags| tags.remove("origin"));
        *priority = origin == metric.tag_value("component_type");
    }

    metrics.sort_unstable_by(|a, b| (&a.0.name, &a.0.tags).cmp(&(&b.0.name, &b.0.tags)));

    // Aggregate same named same tagged metrics.
    metrics.dedup_by(|(metric, metric_priority), (sum, sum_priority)| {
        if (&metric.name, &metric.tags) == (&sum.name, &sum.tags) {
            if let (&MetricValue::Counter { value: a }, &MetricValue::Counter { value: b }) =
                (&metric.value, &sum.value)
            {
                let value = match sum.name.as_str() {
                    // Choose one of the values, where those metrics with
                    // origin same as the components type have an advantage.
                    "events_processed_total" | "processed_bytes_total" => {
                        match (metric_priority, sum_priority) {
                            (true, false) => a,
                            (false, true) => b,
                            // Select max value
                            (true, true) | (false, false) => a.max(b),
                        }
                    }
                    // Sum values
                    _ => a + b,
                };
                sum.value = MetricValue::Counter { value };

                return true;
            }
        }
        false
    });

    out.extend(metrics.drain(..).map(|(m, _)| m));
}

/// Get the events processed by component name.
pub fn component_events_processed_total(component_name: &str) -> Option<EventsProcessedTotal> {
    capture_metrics(&GLOBAL_CONTROLLER)
        .find(|ev| match ev {
            Event::Metric(m)
                if m.name.as_str().eq("events_processed_total")
                    && m.tag_matches("component_name", &component_name) =>
            {
                true
            }
            _ => false,
        })
        .map(|ev| EventsProcessedTotal::new(ev.into_metric()))
}

/// Get the bytes processed by component name.
pub fn component_bytes_processed_total(component_name: &str) -> Option<BytesProcessedTotal> {
    capture_metrics(&GLOBAL_CONTROLLER)
        .find(|ev| match ev {
            Event::Metric(m)
                if m.name.as_str().eq("processed_bytes_total")
                    && m.tag_matches("component_name", &component_name) =>
            {
                true
            }
            _ => false,
        })
        .map(|ev| BytesProcessedTotal::new(ev.into_metric()))
}

type MetricFilterFn = dyn Fn(&Metric) -> bool + Send + Sync;

/// Returns a stream of `Vec<Metric>`, where `metric_name` matches the name of the metric
/// (e.g. "events_processed"), and the value is derived from `MetricValue::Counter`. Uses a
/// local cache to match against the `component_name` of a metric, to return results only when
/// the value of a current iteration is greater than the previous. This is useful for the client
/// to be notified as metrics increase without returning 'empty' or identical results.
pub fn component_counter_metrics(
    interval: i32,
    filter_fn: &'static MetricFilterFn,
) -> impl Stream<Item = Vec<Metric>> {
    let mut cache = BTreeMap::new();

    component_metrics(interval).map(move |m| {
        m.into_iter()
            .filter(filter_fn)
            .filter_map(|m| {
                let component_name = m.tag_value("component_name")?;
                match m.value {
                    MetricValue::Counter { value }
                        if cache.insert(component_name, value).unwrap_or(0.00) < value =>
                    {
                        Some(m)
                    }
                    _ => None,
                }
            })
            .collect()
    })
}

/// Returns the throughput of a 'counter' metric, sampled over `interval` millseconds
/// and filtered by the provided `filter_fn`.
fn counter_throughput(
    interval: i32,
    filter_fn: &'static MetricFilterFn,
) -> impl Stream<Item = (Metric, f64)> {
    let mut last = 0.00;

    get_metrics(interval)
        .filter(filter_fn)
        .filter_map(move |m| match m.value {
            MetricValue::Counter { value } if value > last => {
                let throughput = value - last;
                last = value;
                Some((m, throughput))
            }
            _ => None,
        })
        // Ignore the first, since we only care about sampling between `interval`
        .skip(1)
}

/// Returns the throughput of a 'counter' metric, sampled over `interval` milliseconds
/// and filtered by the provided `filter_fn`, aggregated against each component.
fn component_counter_throughputs(
    interval: i32,
    filter_fn: &'static MetricFilterFn,
) -> impl Stream<Item = Vec<(Metric, f64)>> {
    let mut cache = BTreeMap::new();

    component_metrics(interval)
        .map(move |m| {
            m.into_iter()
                .filter(filter_fn)
                .filter_map(|m| {
                    let component_name = m.tag_value("component_name")?;
                    match m.value {
                        MetricValue::Counter { value } => {
                            let last = cache.insert(component_name, value).unwrap_or(0.00);
                            let throughput = value - last;
                            Some((m, throughput))
                        }
                        _ => None,
                    }
                })
                .collect()
        })
        // Ignore the first, since we only care about sampling between `interval`
        .skip(1)
}

#[cfg(test)]
mod tests {
    use super::aggregate;
    use crate::event::{Metric, MetricKind, MetricValue};

    fn metric(name: &str, tags: Vec<(&str, &str)>, value: f64) -> Metric {
        Metric {
            name: name.into(),
            namespace: None,
            tags: Some(
                tags.into_iter()
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .collect(),
            ),
            value: MetricValue::Counter { value },
            timestamp: None,
            kind: MetricKind::Incremental,
        }
    }

    fn aggregate_test(metrics: Vec<Metric>) -> Vec<Metric> {
        let mut metrics = metrics.into_iter().map(|m| (m, false)).collect();
        let mut out = Vec::new();
        aggregate(&mut metrics, &mut out);
        out
    }

    #[test]
    fn sum() {
        assert_eq!(
            aggregate_test(vec![
                metric(
                    "some_metric",
                    vec![("tag", "value"), ("origin", "test_0")],
                    1.0
                ),
                metric(
                    "some_metric",
                    vec![("tag", "value"), ("origin", "test_1")],
                    1.0
                )
            ]),
            vec![metric("some_metric", vec![("tag", "value")], 2.0)]
        );
    }

    #[test]
    fn choose_eq_type() {
        assert_eq!(
            aggregate_test(vec![
                metric(
                    "events_processed_total",
                    vec![("component_type", "type_0"), ("origin", "type_0")],
                    2.0
                ),
                metric(
                    "events_processed_total",
                    vec![("component_type", "type_0"), ("origin", "type_1")],
                    3.0
                )
            ]),
            vec![metric(
                "events_processed_total",
                vec![("component_type", "type_0")],
                2.0
            )]
        );
    }

    #[test]
    fn choose_neq_type() {
        assert_eq!(
            aggregate_test(vec![
                metric(
                    "events_processed_total",
                    vec![("component_type", "type_0"), ("origin", "type_1")],
                    1.0
                ),
                metric(
                    "events_processed_total",
                    vec![("component_type", "type_0"), ("origin", "type_2")],
                    2.0
                )
            ]),
            vec![metric(
                "events_processed_total",
                vec![("component_type", "type_0")],
                2.0
            )]
        );
    }

    #[test]
    fn multi() {
        assert_eq!(
            aggregate_test(vec![
                metric(
                    "events_processed_total",
                    vec![
                        ("component_type", "type_0"),
                        ("tag", "value"),
                        ("origin", "test_0")
                    ],
                    1.0
                ),
                metric(
                    "events_processed_total",
                    vec![
                        ("component_type", "type_0"),
                        ("tag", "value"),
                        ("origin", "test_1")
                    ],
                    1.0
                ),
                metric(
                    "events_processed_total",
                    vec![("component_type", "type_0"), ("origin", "type_0")],
                    3.0
                ),
                metric(
                    "events_processed_total",
                    vec![("component_type", "type_0"), ("origin", "type_1")],
                    5.0
                ),
                metric(
                    "processed_bytes_total",
                    vec![("component_type", "type_0"), ("origin", "type_1")],
                    1.0
                ),
                metric(
                    "processed_bytes_total",
                    vec![("component_type", "type_0"), ("origin", "type_2")],
                    4.0
                )
            ]),
            vec![
                metric(
                    "events_processed_total",
                    vec![("component_type", "type_0")],
                    3.0
                ),
                metric(
                    "events_processed_total",
                    vec![("component_type", "type_0"), ("tag", "value")],
                    1.0
                ),
                metric(
                    "processed_bytes_total",
                    vec![("component_type", "type_0")],
                    4.0
                )
            ]
        );
    }
}
