//! What a link does to the bytes that cross it.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Whether a link carries bytes at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Bytes cross, shaped.
    #[default]
    Open,
    /// Every byte is held, in both directions and on every connection,
    /// until the link opens again, when it is delivered in order. Nothing
    /// fails, so nothing that waits on the network falls back.
    Stall,
    /// Every connection is reset, open ones at once and new ones as they
    /// arrive, as when a device loses its network.
    Offline,
}

/// The conditions a link imposes, applied to each direction separately.
///
/// Latency, jitter and bandwidth model a path; the rates model faults on
/// it. Rates are chances per segment read (up to 16 KiB), drawn from a
/// generator seeded by [`seed`](Self::seed), so a run with the same
/// traffic draws the same faults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shape {
    /// The profile this shape came from, if any; informational.
    pub name: String,
    pub mode: Mode,
    /// Round trip added to the path. Each direction carries half, and a
    /// new connection pays one more round trip for its handshake before
    /// its first byte reaches the upstream.
    #[serde(with = "millis")]
    pub latency: Duration,
    /// Up to this much more delay on each segment, drawn uniformly. Order
    /// is kept: a segment never overtakes the one before it, as on a TCP
    /// stream.
    #[serde(with = "millis")]
    pub jitter: Duration,
    /// Bandwidth of each direction in kilobits per second, shared by every
    /// connection on the link; 0 is unlimited.
    pub kbps: u64,
    /// Chance a segment holds its direction of a connection for
    /// [`stall_for`](Self::stall_for), as a path that briefly stops
    /// delivering (a handover, a congested queue) does.
    pub stall_rate: f64,
    #[serde(with = "millis")]
    pub stall_for: Duration,
    /// Chance a segment resets its connection.
    pub reset_rate: f64,
    /// A connection that carries nothing either way for this long is
    /// silently forgotten, as by a NAT or a proxy with an idle timeout:
    /// it stays open at both ends and nothing crosses it again, not even
    /// its close. Zero keeps quiet connections.
    #[serde(with = "millis")]
    pub idle_cut: Duration,
    pub seed: u64,
}

impl Default for Shape {
    fn default() -> Self {
        Self {
            name: "lan".into(),
            mode: Mode::Open,
            latency: Duration::ZERO,
            jitter: Duration::ZERO,
            kbps: 0,
            stall_rate: 0.0,
            stall_for: Duration::from_secs(5),
            reset_rate: 0.0,
            idle_cut: Duration::ZERO,
            seed: 0x7012_6b5e_ed00_0001,
        }
    }
}

/// The named profiles, with what each stands for.
pub const PROFILES: [(&str, &str); 7] = [
    ("lan", "no shaping"),
    ("broadband", "30 ms round trip, 5 ms jitter, 20 Mbit/s"),
    ("mobile", "80 ms round trip, 30 ms jitter, 8 Mbit/s"),
    (
        "slow3g",
        "400 ms round trip, 100 ms jitter, 400 kbit/s (Chrome's Slow 3G)",
    ),
    ("satellite", "600 ms round trip, 50 ms jitter, 5 Mbit/s"),
    (
        "flaky",
        "150 ms round trip, 100 ms jitter, 2 Mbit/s, 5 s stalls and resets",
    ),
    (
        "nat",
        "broadband whose quiet connections are forgotten after 30 s",
    ),
];

impl Shape {
    /// The profile `name`, or `None` when there is no such profile.
    pub fn profile(name: &str) -> Option<Self> {
        let path = |latency: u64, jitter: u64, kbps: u64| Self {
            name: name.to_string(),
            latency: Duration::from_millis(latency),
            jitter: Duration::from_millis(jitter),
            kbps,
            ..Self::default()
        };
        Some(match name {
            "lan" => Self::default(),
            "broadband" => path(30, 5, 20_000),
            "mobile" => path(80, 30, 8_000),
            "slow3g" => path(400, 100, 400),
            "satellite" => path(600, 50, 5_000),
            "flaky" => Self {
                stall_rate: 0.002,
                stall_for: Duration::from_secs(5),
                reset_rate: 0.0005,
                ..path(150, 100, 2_000)
            },
            "nat" => Self {
                idle_cut: Duration::from_secs(30),
                ..path(30, 5, 20_000)
            },
            _ => return None,
        })
    }

    /// This shape with each `key=value` in `pairs` applied: `name` (a
    /// profile, applied first), `mode`, `latency_ms`, `jitter_ms`, `kbps`,
    /// `stall_rate`, `stall_ms`, `reset_rate`, `idle_cut_ms`, `seed`, and
    /// `ms`, which sets the latency as `/__gate?mode=latency&ms=` does.
    pub fn with(mut self, pairs: &[(String, String)]) -> Result<Self, String> {
        if let Some((_, name)) = pairs.iter().find(|(key, _)| key == "name") {
            let mode = self.mode;
            self = Self::profile(name).ok_or_else(|| format!("no profile {name:?}"))?;
            self.mode = mode;
        }
        for (key, value) in pairs {
            let millis = || {
                value
                    .parse::<u64>()
                    .map(Duration::from_millis)
                    .map_err(|_| format!("{key}={value} is not a whole number of ms"))
            };
            let rate = || {
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|rate| (0.0..=1.0).contains(rate))
                    .ok_or_else(|| format!("{key}={value} is not a chance between 0 and 1"))
            };
            match key.as_str() {
                "name" | "link" => {}
                "mode" => {
                    self.mode = match value.as_str() {
                        "open" => Mode::Open,
                        // gate.mjs's latency mode is an open link with a
                        // round trip, which `ms` sets.
                        "latency" => Mode::Open,
                        "stall" => Mode::Stall,
                        "offline" => Mode::Offline,
                        other => return Err(format!("no mode {other:?}")),
                    }
                }
                "latency_ms" | "ms" => self.latency = millis()?,
                "jitter_ms" => self.jitter = millis()?,
                "kbps" => {
                    self.kbps = value
                        .parse()
                        .map_err(|_| format!("kbps={value} is not a number"))?
                }
                "stall_rate" => self.stall_rate = rate()?,
                "stall_ms" => self.stall_for = millis()?,
                "reset_rate" => self.reset_rate = rate()?,
                "idle_cut_ms" => self.idle_cut = millis()?,
                "seed" => {
                    self.seed = value
                        .parse()
                        .map_err(|_| format!("seed={value} is not a number"))?
                }
                other => return Err(format!("unknown setting {other:?}")),
            }
        }
        Ok(self)
    }

    /// How long `bytes` take to cross at this shape's bandwidth.
    pub(crate) fn transmit(&self, bytes: usize) -> Duration {
        if self.kbps == 0 {
            return Duration::ZERO;
        }
        // kbit/s is bits per ms.
        Duration::from_micros(bytes as u64 * 8 * 1000 / self.kbps)
    }
}

/// A small deterministic generator for jitter and faults: xorshift64*.
#[derive(Debug, Clone)]
pub(crate) struct Draw(u64);

impl Draw {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    /// A number in `[0, 1)`.
    pub(crate) fn unit(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let value = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d);
        (value >> 11) as f64 / (1u64 << 53) as f64
    }

    pub(crate) fn chance(&mut self, rate: f64) -> bool {
        rate > 0.0 && self.unit() < rate
    }

    pub(crate) fn upto(&mut self, limit: Duration) -> Duration {
        limit.mul_f64(self.unit())
    }
}

mod millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(value.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        u64::deserialize(deserializer).map(Duration::from_millis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_profile_exists() {
        for (name, _) in PROFILES {
            assert!(Shape::profile(name).is_some(), "{name}");
        }
        assert!(Shape::profile("dialup").is_none());
    }

    #[test]
    fn a_gate_query_sets_mode_and_latency() {
        let pairs = |items: &[(&str, &str)]| {
            items
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<Vec<_>>()
        };
        let shape = Shape::default()
            .with(&pairs(&[("mode", "latency"), ("ms", "2000")]))
            .unwrap();
        assert_eq!(shape.mode, Mode::Open);
        assert_eq!(shape.latency, Duration::from_secs(2));

        let stalled = Shape::profile("mobile")
            .unwrap()
            .with(&pairs(&[("mode", "stall")]))
            .unwrap();
        assert_eq!(stalled.mode, Mode::Stall);
        assert_eq!(stalled.kbps, 8_000, "a mode switch keeps the path");

        let renamed = stalled.with(&pairs(&[("name", "lan")])).unwrap();
        assert_eq!(renamed.mode, Mode::Stall, "a profile switch keeps the mode");
        assert_eq!(renamed.kbps, 0);

        assert!(
            Shape::default()
                .with(&pairs(&[("reset_rate", "2")]))
                .is_err()
        );
        assert!(Shape::default().with(&pairs(&[("colour", "red")])).is_err());
    }

    #[test]
    fn transmit_time_follows_bandwidth() {
        let shape = Shape {
            kbps: 8,
            ..Shape::default()
        };
        assert_eq!(shape.transmit(1000), Duration::from_secs(1));
        assert_eq!(Shape::default().transmit(1 << 20), Duration::ZERO);
    }

    #[test]
    fn draws_repeat_for_a_seed() {
        let mut first = Draw::new(7);
        let mut second = Draw::new(7);
        for _ in 0..100 {
            let value = first.unit();
            assert_eq!(value, second.unit());
            assert!((0.0..1.0).contains(&value));
        }
    }
}
