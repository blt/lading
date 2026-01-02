//! Syslog payload.

use std::{io::Write, time::SystemTime};

use rand::{Rng, distr::StandardUniform, prelude::Distribution, seq::IndexedRandom};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::Error;

#[derive(Debug, Default, Clone, Copy)]
#[allow(clippy::module_name_repetitions)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
/// Syslog 5424 payload
pub struct Syslog5424 {}

const HOSTNAMES: [&str; 4] = [
    "troutwine.us",
    "vector.dev",
    "skullmountain.us",
    "zombo.com",
];
const APP_NAMES: [&str; 4] = ["vector", "cernan", "lading", "execsnoop"];

#[derive(serde::Serialize)]
struct Message {
    eccentricity: u32,
    semimajor_axis: u32,
    inclination: f32,
    longitude_of_ascending_node: u32,
    periapsis: u64,
    true_anomaly: u8,
}

impl Distribution<Message> for StandardUniform {
    fn sample<R>(&self, rng: &mut R) -> Message
    where
        R: Rng + ?Sized,
    {
        Message {
            eccentricity: rng.random(),
            semimajor_axis: rng.random(),
            inclination: rng.random(),
            longitude_of_ascending_node: rng.random(),
            periapsis: rng.random(),
            true_anomaly: rng.random(),
        }
    }
}

struct Member {
    priority: u8,           // 0 - 191
    syslog_version: u8,     // 1 - 3
    timestamp: String,      // seconds format in millis
    hostname: &'static str, // name.tld
    app_name: &'static str, // shortish string
    procid: u16,            // 100 - 9999
    msgid: u16,             // 1 - 999
    message: Message,       // structured data, serialized on write
}

impl Distribution<Member> for StandardUniform {
    fn sample<R>(&self, rng: &mut R) -> Member
    where
        R: Rng + ?Sized,
    {
        Member {
            priority: rng.random_range(0..=191),
            syslog_version: rng.random_range(1..=3),
            timestamp: to_rfc3339(SystemTime::now()),
            hostname: HOSTNAMES.choose(rng).expect("failed to choose hostname"),
            app_name: APP_NAMES.choose(rng).expect("failed to choose app name"),
            procid: rng.random_range(100..=9999),
            msgid: rng.random_range(1..=999),
            message: rng.random(),
        }
    }
}

fn to_rfc3339<T>(dt: T) -> String
where
    T: Into<OffsetDateTime>,
{
    dt.into().format(&Rfc3339).expect("failed to format")
}

impl Member {
    /// Write the member to a buffer.
    fn write_to(&self, buffer: &mut Vec<u8>) -> Result<(), Error> {
        write!(
            buffer,
            "<{priority}>{version} {timestamp} {hostname} {app_name} {procid} ID{msgid} - ",
            priority = self.priority,
            version = self.syslog_version,
            timestamp = self.timestamp,
            hostname = self.hostname,
            app_name = self.app_name,
            procid = self.procid,
            msgid = self.msgid,
        )
        .expect("formatting to Vec<u8> cannot fail");
        serde_json::to_writer(&mut *buffer, &self.message)?;
        Ok(())
    }
}

impl crate::Serialize for Syslog5424 {
    fn to_bytes<W, R>(&mut self, rng: R, max_bytes: usize, writer: &mut W) -> Result<(), Error>
    where
        R: Rng + Sized,
        W: Write,
    {
        if max_bytes < 2 {
            // 'empty' payload  is []
            return Ok(());
        }

        let mut bytes_remaining = max_bytes;
        // Reuse a single buffer across iterations to avoid repeated allocations.
        // Each log line is formatted here, measured, then written to the output.
        let mut buffer: Vec<u8> = Vec::with_capacity(512);
        for member in rng.sample_iter::<Member, StandardUniform>(StandardUniform) {
            buffer.clear();
            member.write_to(&mut buffer)?;
            let line_length = buffer.len() + 1; // add one for the newline

            match bytes_remaining.checked_sub(line_length) {
                Some(remainder) => {
                    writer.write_all(&buffer)?;
                    writer.write_all(b"\n")?;
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
    use rand::{SeedableRng, rngs::SmallRng};

    use crate::{Serialize, Syslog5424};

    // We want to be sure that the serialized size of the payload does not
    // exceed `max_bytes`.
    proptest! {
        #[test]
        fn payload_not_exceed_max_bytes(seed: u64, max_bytes: u16) {
            let max_bytes = max_bytes as usize;
            let rng = SmallRng::seed_from_u64(seed);
            let mut syslog = Syslog5424::default();

            let mut bytes = Vec::with_capacity(max_bytes);
            syslog.to_bytes(rng, max_bytes, &mut bytes).expect("failed to convert to bytes");
            debug_assert!(
                bytes.len() <= max_bytes,
                "{:?}",
                std::str::from_utf8(&bytes).expect("failed to convert from utf-8 to str")
            );
        }
    }
}
