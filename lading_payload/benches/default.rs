//! Main entry point for lading_payload benchmarks.

use criterion::criterion_main;

mod apache_common;
mod ascii;
mod dogstatsd;
mod fluent;
mod opentelemetry_log;
mod opentelemetry_metric;
mod opentelemetry_traces;
mod trace_agent;

criterion_main!(
    apache_common::benches,
    ascii::benches,
    dogstatsd::benches,
    fluent::benches,
    opentelemetry_log::benches,
    opentelemetry_metric::benches,
    opentelemetry_traces::benches,
    trace_agent::benches,
);
