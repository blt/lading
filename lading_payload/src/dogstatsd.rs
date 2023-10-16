//! `DogStatsD` payload.

use std::{cmp, fmt, io::Write, ops::Range, rc::Rc};

use rand::{
    distributions::{uniform::SampleUniform, WeightedIndex},
    prelude::Distribution,
    seq::SliceRandom,
    Rng,
};
use serde::Deserialize;

use crate::{common::strings, Error, Serialize};

use self::{
    common::tags, event::EventGenerator, metric::MetricGenerator,
    service_check::ServiceCheckGenerator,
};

use super::Generator;

mod common;
mod event;
mod metric;
mod service_check;

fn contexts() -> ConfRange<u32> {
    ConfRange::Inclusive {
        min: 5_000,
        max: 10_000,
    }
}

fn value_config() -> ValueConf {
    ValueConf {
        float_probability: 0.5, // 50%
        range: ConfRange::Inclusive {
            min: i64::MIN,
            max: i64::MAX,
        },
    }
}

// https://docs.datadoghq.com/developers/guide/what-best-practices-are-recommended-for-naming-metrics-and-tags/#rules-and-best-practices-for-naming-metrics
fn name_length() -> ConfRange<u16> {
    ConfRange::Inclusive { min: 1, max: 200 }
}

fn tag_key_length() -> ConfRange<u16> {
    ConfRange::Inclusive { min: 1, max: 100 }
}

fn tag_value_length() -> ConfRange<u16> {
    ConfRange::Inclusive { min: 1, max: 100 }
}

fn tags_per_msg() -> ConfRange<u16> {
    ConfRange::Inclusive { min: 2, max: 50 }
}

fn multivalue_count() -> ConfRange<u16> {
    ConfRange::Inclusive { min: 2, max: 32 }
}

fn multivalue_pack_probability() -> f32 {
    0.08
}

/// Weights for `DogStatsD` kinds: metrics, events, service checks
///
/// Defines the relative probability of each kind of `DogStatsD` datagram.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct KindWeights {
    metric: u8,
    event: u8,
    service_check: u8,
}

impl Default for KindWeights {
    fn default() -> Self {
        KindWeights {
            metric: 80,        // 80%
            event: 10,         // 10%
            service_check: 10, // 10%
        }
    }
}

/// Weights for `DogStatsD` metrics: gauges, counters, etc
#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct MetricWeights {
    count: u8,
    gauge: u8,
    timer: u8,
    distribution: u8,
    set: u8,
    histogram: u8,
}

impl Default for MetricWeights {
    fn default() -> Self {
        MetricWeights {
            count: 34,       // 34%
            gauge: 34,       // 34%
            timer: 5,        // 5%
            distribution: 1, // 1%
            set: 1,          // 1%
            histogram: 25,   // 25%
        }
    }
}

/// Configuration for the values of a metric.
#[derive(Debug, Deserialize, Clone, PartialEq, Copy)]
pub struct ValueConf {
    /// Odds out of 256 that the value will be a float and not an integer.
    float_probability: f32,
    range: ConfRange<i64>,
}

/// Range expression for configuration
#[derive(Debug, Deserialize, Clone, PartialEq, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ConfRange<T>
where
    T: PartialEq + cmp::PartialOrd + Clone + Copy,
{
    /// A constant T
    Constant(T),
    /// In which a T is chosen between `min` and `max`, inclusive of `max`.
    Inclusive {
        /// The minimum of the range.
        min: T,
        /// The maximum of the range.
        max: T,
    },
}

impl<T> ConfRange<T>
where
    T: PartialEq + cmp::PartialOrd + Clone + Copy,
{
    fn end(&self) -> T {
        match self {
            ConfRange::Constant(c) => *c,
            ConfRange::Inclusive { max, .. } => *max,
        }
    }
}

impl<T> ConfRange<T>
where
    T: PartialEq + cmp::PartialOrd + Clone + Copy + SampleUniform,
{
    fn sample<R>(&self, rng: &mut R) -> T
    where
        R: rand::Rng + ?Sized,
    {
        match self {
            ConfRange::Constant(c) => *c,
            ConfRange::Inclusive { min, max } => rng.gen_range(*min..*max),
        }
    }
}

/// Configure the `DogStatsD` payload.
#[derive(Debug, Deserialize, Clone, PartialEq, Copy)]
pub struct Config {
    /// The unique metric contexts to generate A context is a set of unique
    /// metric name + tags
    #[serde(default = "contexts")]
    pub contexts: ConfRange<u32>,

    /// Length for a dogstatsd message name
    #[serde(default = "name_length")]
    pub name_length: ConfRange<u16>,

    /// Length for the 'key' part of a dogstatsd tag
    #[serde(default = "tag_key_length")]
    pub tag_key_length: ConfRange<u16>,

    /// Length for the 'value' part of a dogstatsd tag
    #[serde(default = "tag_value_length")]
    pub tag_value_length: ConfRange<u16>,

    /// Number of tags per individual dogstatsd msg a tag is a key-value pair
    /// separated by a :
    #[serde(default = "tags_per_msg")]
    pub tags_per_msg: ConfRange<u16>,

    /// Probability between 0 and 1 that a given dogstatsd msg
    /// contains multiple values
    #[serde(default = "multivalue_pack_probability")]
    pub multivalue_pack_probability: f32,

    /// The count of values that will be generated if multi-value is chosen to
    /// be generated
    #[serde(default = "multivalue_count")]
    pub multivalue_count: ConfRange<u16>,

    /// Defines the relative probability of each kind of DogStatsD kinds of
    /// payload.
    #[serde(default)]
    pub kind_weights: KindWeights,

    /// Defines the relative probability of each kind of DogStatsD metric.
    #[serde(default)]
    pub metric_weights: MetricWeights,

    /// The configuration of values that appear in all metrics.
    #[serde(default = "value_config")]
    pub value: ValueConf,
}

fn choose_or_not_ref<'a, R, T>(mut rng: &mut R, pool: &'a [T]) -> Option<&'a T>
where
    R: rand::Rng + ?Sized,
{
    if rng.gen() {
        pool.choose(&mut rng)
    } else {
        None
    }
}

fn choose_or_not_fn<R, T, F>(rng: &mut R, func: F) -> Option<T>
where
    T: Clone,
    R: rand::Rng + ?Sized,
    F: FnOnce(&mut R) -> Option<T>,
{
    if rng.gen() {
        func(rng)
    } else {
        None
    }
}

#[inline]
/// Generate a total number of strings randomly chosen from the range `min_max` with a maximum length
/// per string of `max_length`.
fn random_strings_with_length<R>(
    pool: &strings::Pool,
    min_max: Range<usize>,
    max_length: u16,
    rng: &mut R,
) -> Vec<String>
where
    R: Rng + ?Sized,
{
    let total = rng.gen_range(min_max);
    let length_range = ConfRange::Inclusive {
        min: 1,
        max: max_length,
    };

    random_strings_with_length_range(pool, total, length_range, rng)
}

#[inline]
/// Generate a `total` number of strings with a maximum length per string of
/// `max_length`.
fn random_strings_with_length_range<R>(
    pool: &strings::Pool,
    total: usize,
    length_range: ConfRange<u16>,
    mut rng: &mut R,
) -> Vec<String>
where
    R: Rng + ?Sized,
{
    let mut buf = Vec::with_capacity(total);
    for _ in 0..total {
        let sz = length_range.sample(&mut rng) as usize;
        buf.push(String::from(pool.of_size(&mut rng, sz).unwrap()));
    }
    buf
}

#[derive(Debug, Clone)]
struct MemberGenerator {
    kind_weights: WeightedIndex<u8>,
    event_generator: EventGenerator,
    service_check_generator: ServiceCheckGenerator,
    metric_generator: MetricGenerator,
}

impl MemberGenerator {
    #[allow(clippy::too_many_arguments)]
    fn new<R>(
        contexts: ConfRange<u32>,
        name_length: ConfRange<u16>,
        tag_key_length: ConfRange<u16>,
        tag_value_length: ConfRange<u16>,
        tags_per_msg: ConfRange<u16>,
        multivalue_count: ConfRange<u16>,
        multivalue_pack_probability: f32,
        kind_weights: KindWeights,
        metric_weights: MetricWeights,
        value_conf: ValueConf,
        mut rng: &mut R,
    ) -> Self
    where
        R: Rng + ?Sized,
    {
        let pool = Rc::new(strings::Pool::with_size(&mut rng, 8_000_000));

        let num_contexts = contexts.sample(rng);

        let tags_generator = tags::Generator {
            num_tagsets: num_contexts as usize,
            tags_per_msg,
            tag_key_length,
            tag_value_length,
            str_pool: Rc::clone(&pool),
        };

        let service_event_titles = random_strings_with_length_range(
            pool.as_ref(),
            num_contexts as usize,
            name_length,
            &mut rng,
        );
        let tagsets = tags_generator.generate(&mut rng);
        drop(tags_generator); // now unused, clear up its allocation s

        let texts_or_messages = random_strings_with_length(pool.as_ref(), 4..128, 1024, &mut rng);
        let small_strings = random_strings_with_length(pool.as_ref(), 16..1024, 8, &mut rng);

        // NOTE the ordering here of `metric_choices` is very important! If you
        // change it here you MUST also change it in `Generator<Metric> for
        // MetricGenerator`.
        let metric_choices = [
            metric_weights.count,
            metric_weights.gauge,
            metric_weights.timer,
            metric_weights.distribution,
            metric_weights.set,
            metric_weights.histogram,
        ];

        let event_generator = EventGenerator {
            str_pool: Rc::clone(&pool),
            title_length: name_length,
            texts_or_messages_length_range: 1..1024,
            small_strings_length_range: 1..8,
            tagsets: tagsets.clone(),
        };

        let service_check_generator = ServiceCheckGenerator {
            names: service_event_titles.clone(),
            small_strings: small_strings.clone(),
            texts_or_messages,
            tagsets: tagsets.clone(),
        };

        let metric_generator = MetricGenerator::new(
            num_contexts as usize,
            name_length,
            multivalue_count,
            multivalue_pack_probability,
            &WeightedIndex::new(metric_choices).unwrap(),
            small_strings,
            tagsets.clone(),
            pool.as_ref(),
            value_conf,
            &mut rng,
        );

        // NOTE the ordering here of `member_choices` is very important! If you
        // change it here you MUST also change it in `Generator<Member> for
        // MemberGenerator`.
        let member_choices = [
            kind_weights.metric,
            kind_weights.event,
            kind_weights.service_check,
        ];
        MemberGenerator {
            kind_weights: WeightedIndex::new(member_choices).unwrap(),
            event_generator,
            service_check_generator,
            metric_generator,
        }
    }
}

impl MemberGenerator {
    fn generate<'a, R>(&'a self, rng: &mut R) -> Member<'a>
    where
        R: rand::Rng + ?Sized,
    {
        match self.kind_weights.sample(rng) {
            0 => Member::Metric(self.metric_generator.generate(rng)),
            1 => {
                let event = self.event_generator.generate(rng);
                Member::Event(event)
            }
            2 => Member::ServiceCheck(self.service_check_generator.generate(rng)),
            _ => unreachable!(),
        }
    }
}

// https://docs.datadoghq.com/developers/dogstatsd/datagram_shell/
#[derive(Debug)]
/// Supra-type for all dogstatsd variants
pub enum Member<'a> {
    /// Metrics
    Metric(metric::Metric<'a>),
    /// Events, think syslog.
    Event(event::Event<'a>),
    /// Services, checked.
    ServiceCheck(service_check::ServiceCheck<'a>),
}

impl<'a> fmt::Display for Member<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Metric(ref m) => write!(f, "{m}"),
            Self::Event(ref e) => write!(f, "{e}"),
            Self::ServiceCheck(ref sc) => write!(f, "{sc}"),
        }
    }
}

#[derive(Debug, Clone)]
#[allow(clippy::module_name_repetitions)]
/// A generator for `DogStatsD` payloads
pub struct DogStatsD {
    member_generator: MemberGenerator,
}

impl DogStatsD {
    /// Create a new default instance of `DogStatsD` with reasonable settings.
    pub fn default<R>(rng: &mut R) -> Self
    where
        R: rand::Rng + ?Sized,
    {
        Self::new(
            contexts(),
            name_length(),
            tag_key_length(),
            tag_value_length(),
            tags_per_msg(),
            multivalue_count(),
            multivalue_pack_probability(),
            KindWeights::default(),
            MetricWeights::default(),
            value_config(),
            rng,
        )
    }

    #[cfg(feature = "dogstatsd_perf")]
    /// Call the internal member generator and count the in-memory byte
    /// size. This is not useful except in a loop to track how quickly we can do
    /// this operation. It's meant to be a proxy by which we can determine how
    /// quickly members are able to be generated and then serialized. An
    /// approximation.
    pub fn generate<R>(&self, rng: &mut R) -> Member
    where
        R: rand::Rng + ?Sized,
    {
        self.member_generator.generate(rng)
    }

    /// Create a new instance of `DogStatsD`.
    #[allow(clippy::too_many_arguments)]
    pub fn new<R>(
        contexts: ConfRange<u32>,
        name_length: ConfRange<u16>,
        tag_key_length: ConfRange<u16>,
        tag_value_length: ConfRange<u16>,
        tags_per_msg: ConfRange<u16>,
        multivalue_count: ConfRange<u16>,
        multivalue_pack_probability: f32,
        kind_weights: KindWeights,
        metric_weights: MetricWeights,
        value_conf: ValueConf,
        rng: &mut R,
    ) -> Self
    where
        R: rand::Rng + ?Sized,
    {
        let member_generator = MemberGenerator::new(
            contexts,
            name_length,
            tag_key_length,
            tag_value_length,
            tags_per_msg,
            multivalue_count,
            multivalue_pack_probability,
            kind_weights,
            metric_weights,
            value_conf,
            rng,
        );

        Self { member_generator }
    }
}

impl Serialize for DogStatsD {
    fn to_bytes<W, R>(&self, mut rng: R, max_bytes: usize, writer: &mut W) -> Result<(), Error>
    where
        R: Rng + Sized,
        W: Write,
    {
        let mut bytes_remaining = max_bytes;
        loop {
            let member: Member = self.member_generator.generate(&mut rng);
            let encoding = format!("{member}");
            let line_length = encoding.len() + 1; // add one for the newline
            match bytes_remaining.checked_sub(line_length) {
                Some(remainder) => {
                    writeln!(writer, "{encoding}")?;
                    bytes_remaining = remainder;
                }
                None => break,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use proptest::prelude::*;
    use rand::{rngs::SmallRng, SeedableRng};

    use crate::{
        dogstatsd::{
            contexts, multivalue_count, multivalue_pack_probability, name_length, tag_key_length,
            tag_value_length, tags_per_msg, value_config, KindWeights, MetricWeights,
        },
        DogStatsD, Serialize,
    };

    // We want to be sure that the serialized size of the payload does not
    // exceed `max_bytes`.
    proptest! {
        #[test]
        fn payload_not_exceed_max_bytes(seed: u64, max_bytes: u16) {
            let max_bytes = max_bytes as usize;
            let mut rng = SmallRng::seed_from_u64(seed);
            let multivalue_pack_probability = multivalue_pack_probability();
            let value_conf = value_config();

            let kind_weights = KindWeights::default();
            let metric_weights = MetricWeights::default();
            let dogstatsd = DogStatsD::new(contexts(), name_length(), tag_key_length(),
                                           tag_value_length(), tags_per_msg(),
                                           multivalue_count(), multivalue_pack_probability, kind_weights,
                                           metric_weights, value_conf, &mut rng);

            let mut bytes = Vec::with_capacity(max_bytes);
            dogstatsd.to_bytes(rng, max_bytes, &mut bytes).unwrap();
            debug_assert!(
                bytes.len() <= max_bytes,
                "{:?}",
                std::str::from_utf8(&bytes).unwrap()
            );
        }
    }
}
