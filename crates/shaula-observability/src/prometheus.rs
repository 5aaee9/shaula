//! Prometheus text exposition (format 0.0.4) of bounded process metrics.
//!
//! Every label value passes [`label_value`]: fixed codes only, so series
//! cardinality stays bounded and no identifier, URL or secret can become a
//! label. Generation and job IDs are never labels.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Mutex, PoisonError};

/// Upper bounds, in seconds, of IaC effect duration buckets.
pub const DURATION_BUCKETS: [f64; 11] = [
    1.0, 5.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1200.0, 1800.0, 3600.0,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Counter,
    Gauge,
    Histogram,
}

impl Kind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

/// One metric family: its exposition name, help text and type.
#[derive(Debug)]
pub struct Family {
    pub name: &'static str,
    pub help: &'static str,
    pub kind: Kind,
}

type Labels = Vec<(&'static str, String)>;

#[derive(Default)]
struct Histogram {
    buckets: [u64; DURATION_BUCKETS.len()],
    sum: f64,
    count: u64,
}

#[derive(Default)]
struct Registry {
    families: BTreeMap<&'static str, &'static Family>,
    counters: BTreeMap<(&'static str, Labels), u64>,
    histograms: BTreeMap<(&'static str, Labels), Histogram>,
}

static REGISTRY: Mutex<Option<Registry>> = Mutex::new(None);

fn with_registry<T>(f: impl FnOnce(&mut Registry) -> T) -> T {
    let mut guard = REGISTRY.lock().unwrap_or_else(PoisonError::into_inner);
    f(guard.get_or_insert_with(Registry::default))
}

fn owned(labels: &[(&'static str, &str)]) -> Labels {
    labels
        .iter()
        .map(|(name, value)| (*name, label_value(value)))
        .collect()
}

/// A fixed-code label value: 1..=64 characters of `[A-Za-z0-9_.:-]`.
/// Anything else (free text, URLs, paths) collapses to `other`.
pub fn label_value(value: &str) -> String {
    let fixed = !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'));
    if fixed {
        value.to_owned()
    } else {
        "other".to_owned()
    }
}

/// Adds `delta` to a process-lifetime counter.
pub fn count(family: &'static Family, labels: &[(&'static str, &str)], delta: u64) {
    with_registry(|registry| {
        registry.families.insert(family.name, family);
        *registry
            .counters
            .entry((family.name, owned(labels)))
            .or_default() += delta;
    });
}

/// Records one observation, in seconds, into a [`DURATION_BUCKETS`] histogram.
pub fn observe(family: &'static Family, labels: &[(&'static str, &str)], seconds: f64) {
    with_registry(|registry| {
        registry.families.insert(family.name, family);
        let histogram = registry
            .histograms
            .entry((family.name, owned(labels)))
            .or_default();
        for (slot, bound) in DURATION_BUCKETS.iter().enumerate() {
            if seconds <= *bound {
                histogram.buckets[slot] += 1;
            }
        }
        histogram.sum += seconds;
        histogram.count += 1;
    });
}

/// A scrape body under construction: scrape-time gauges plus every
/// recorded counter and histogram, grouped by family.
#[derive(Default)]
pub struct Exposition {
    families: BTreeMap<&'static str, (&'static Family, Vec<String>)>,
}

impl Exposition {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one scrape-time sample (a gauge, or a counter owned elsewhere).
    pub fn sample(&mut self, family: &'static Family, labels: &[(&'static str, &str)], value: f64) {
        let line = format!(
            "{}{} {}",
            family.name,
            render_labels(&owned(labels)),
            number(value)
        );
        self.lines(family).push(line);
    }

    fn lines(&mut self, family: &'static Family) -> &mut Vec<String> {
        &mut self
            .families
            .entry(family.name)
            .or_insert_with(|| (family, Vec::new()))
            .1
    }

    /// Renders the body, including the process-lifetime registry.
    pub fn finish(mut self) -> String {
        with_registry(|registry| {
            for ((name, labels), value) in &registry.counters {
                let family = registry.families[name];
                let line = format!("{name}{} {value}", render_labels(labels));
                self.lines(family).push(line);
            }
            for ((name, labels), histogram) in &registry.histograms {
                let family = registry.families[name];
                let lines = histogram_lines(name, labels, histogram);
                self.lines(family).extend(lines);
            }
        });
        let mut body = String::new();
        for (name, (family, lines)) in &self.families {
            let _ = writeln!(body, "# HELP {name} {}", family.help);
            let _ = writeln!(body, "# TYPE {name} {}", family.kind.as_str());
            for line in lines {
                body.push_str(line);
                body.push('\n');
            }
        }
        body
    }
}

fn histogram_lines(name: &str, labels: &Labels, histogram: &Histogram) -> Vec<String> {
    let mut lines = Vec::new();
    for (slot, bound) in DURATION_BUCKETS.iter().enumerate() {
        let mut bucket = labels.clone();
        bucket.push(("le", number(*bound)));
        lines.push(format!(
            "{name}_bucket{} {}",
            render_labels(&bucket),
            histogram.buckets[slot]
        ));
    }
    let mut infinite = labels.clone();
    infinite.push(("le", "+Inf".to_owned()));
    lines.push(format!(
        "{name}_bucket{} {}",
        render_labels(&infinite),
        histogram.count
    ));
    lines.push(format!(
        "{name}_sum{} {}",
        render_labels(labels),
        number(histogram.sum)
    ));
    lines.push(format!(
        "{name}_count{} {}",
        render_labels(labels),
        histogram.count
    ));
    lines
}

fn render_labels(labels: &Labels) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let rendered: Vec<String> = labels
        .iter()
        .map(|(name, value)| {
            let escaped = value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n");
            format!("{name}=\"{escaped}\"")
        })
        .collect();
    format!("{{{}}}", rendered.join(","))
}

fn number(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}
