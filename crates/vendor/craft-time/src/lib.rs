// Vendored from storytold/craft-libs `crates/time/src/lib.rs` at
// a4db799563c9ccd7217e1731ec34c58f6ba57248 (the revision FilmCraft 2d1d1b6 pins).
// Copyright (c) 2026 ArtCraft Team and the craft-libs contributors.
// Used under the MIT licence (see LICENSE-MIT). Trimmed by Vin's Atelier: kept Tick, TimeRange and
// FrameRate verbatim; removed the soundcraft module, timecode display/parsing and the tests.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Exact media time for the Craft suite.
//!
//! All time is an integer number of [`Tick`]s at [`TICKS_PER_SECOND`] = 254 016 000 000/s. That
//! rate divides evenly into every broadcast frame duration (23.976, 24, 25, 29.97, 30, 48, 50,
//! 59.94, 60, 119.88, 120 fps) and standard audio sample rates (44.1 kHz, 48 kHz, and their
//! multiples), so edits, frame math, audio alignment, and visual effects never drift.
//!
//! Timecode (SMPTE drop/non-drop, frames, feet+frames, audio samples) is a display representation
//! of ticks. Every conversion is total: invalid, zero, or negative rates produce well-defined defaults
//! rather than division-by-zero panics.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Rem, Sub, SubAssign};

/// Clock ticks per second: 254 016 000 000.
pub const TICKS_PER_SECOND: i64 = 254_016_000_000;

/// A point or duration in time, in ticks.
/// Arithmetic saturates at storage limits; representable results remain exact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tick(pub i64);

impl Tick {
    pub const ZERO: Tick = Tick(0);
    pub const MAX: Tick = Tick(i64::MAX / 4);
    pub const MIN: Tick = Tick(i64::MIN / 4);

    /// Seconds to ticks, clamped to [`Tick::MIN`]..=[`Tick::MAX`].
    pub fn from_seconds_f64(s: f64) -> Tick {
        Tick((s * TICKS_PER_SECOND as f64).round() as i64)
    }

    pub fn seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// Exact conversion from a count of `units` at `per_second` (e.g. samples at 48 000).
    pub fn from_units(units: i64, per_second: i64) -> Tick {
        Tick(clamp_i64((units as i128 * TICKS_PER_SECOND as i128).checked_div(per_second as i128).unwrap_or(0)))
    }

    /// Floor conversion to a count of units at `per_second`.
    pub fn to_units_floor(self, per_second: i64) -> i64 {
        clamp_i64((self.0 as i128 * per_second as i128).div_euclid(TICKS_PER_SECOND as i128))
    }

    /// Conversion from a rational timestamp `pts * num / den` seconds (container timebases).
    pub fn from_rational(pts: i64, num: i64, den: i64) -> Tick {
        if den == 0 {
            return Tick::ZERO;
        }
        let Some(t) = (pts as i128 * num as i128).checked_mul(TICKS_PER_SECOND as i128) else {
            return Tick(if (pts < 0) ^ (num < 0) ^ (den < 0) { i64::MIN } else { i64::MAX });
        };
        Tick(clamp_i64(t.div_euclid(den as i128)))
    }

    /// Inverse of [`Tick::from_rational`], floored to the timebase.
    pub fn to_rational_floor(self, num: i64, den: i64) -> i64 {
        clamp_i64((self.0 as i128 * den as i128).checked_div_euclid(num as i128 * TICKS_PER_SECOND as i128).unwrap_or(0))
    }

    /// Inverse of [`Tick::from_rational`], rounded to the nearest timebase unit (halves up).
    pub fn to_rational_round(self, num: i64, den: i64) -> i64 {
        let n = self.0 as i128 * den as i128;
        let unit = num as i128 * TICKS_PER_SECOND as i128;
        let (n, unit) = if unit < 0 { (-n, -unit) } else { (n, unit) };
        let (Some(q), Some(r)) = (n.checked_div_euclid(unit), n.checked_rem_euclid(unit)) else { return 0 };
        clamp_i64(if r * 2 >= unit { q + 1 } else { q })
    }

    pub fn abs(self) -> Tick {
        Tick(self.0.saturating_abs())
    }

    pub fn signum(self) -> i64 {
        self.0.signum()
    }

    pub fn min(self, o: Tick) -> Tick {
        Tick(self.0.min(o.0))
    }

    pub fn max(self, o: Tick) -> Tick {
        Tick(self.0.max(o.0))
    }

    pub fn clamp(self, lo: Tick, hi: Tick) -> Tick {
        Tick(self.0.max(lo.0).min(hi.0))
    }

    /// Scale by a rational factor (`num/den`), flooring.
    pub fn mul_ratio(self, num: i64, den: i64) -> Tick {
        Tick(clamp_i64((self.0 as i128 * num as i128).checked_div_euclid(den as i128).unwrap_or(0)))
    }

    /// Divide by a rational factor (`num/den`), flooring.
    pub fn div_ratio(self, num: i64, den: i64) -> Tick {
        self.mul_ratio(den, num)
    }

    /// Checked addition. Returns `None` if overflow occurred.
    pub fn checked_add(self, rhs: Tick) -> Option<Tick> {
        self.0.checked_add(rhs.0).map(Tick)
    }

    /// Checked subtraction. Returns `None` if overflow occurred.
    pub fn checked_sub(self, rhs: Tick) -> Option<Tick> {
        self.0.checked_sub(rhs.0).map(Tick)
    }

    /// Checked multiplication. Returns `None` if overflow occurred.
    pub fn checked_mul(self, rhs: i64) -> Option<Tick> {
        self.0.checked_mul(rhs).map(Tick)
    }

    /// Checked division. Returns `None` if `rhs == 0` or if overflow occurred (`Tick(i64::MIN) / -1`).
    pub fn checked_div(self, rhs: i64) -> Option<Tick> {
        self.0.checked_div(rhs).map(Tick)
    }

    /// Checked remainder. Returns `None` if `rhs.0 == 0` or if overflow occurred (`Tick(i64::MIN) % Tick(-1)`).
    pub fn checked_rem(self, rhs: Tick) -> Option<Tick> {
        self.0.checked_rem(rhs.0).map(Tick)
    }

    /// Saturating addition.
    pub fn saturating_add(self, rhs: Tick) -> Tick {
        Tick(self.0.saturating_add(rhs.0))
    }

    /// Saturating subtraction.
    pub fn saturating_sub(self, rhs: Tick) -> Tick {
        Tick(self.0.saturating_sub(rhs.0))
    }

    /// Saturating multiplication.
    pub fn saturating_mul(self, rhs: i64) -> Tick {
        Tick(self.0.saturating_mul(rhs))
    }

    /// Saturating division. Returns `None` if `rhs == 0`, and saturates on overflow (`Tick(i64::MIN) / -1 -> Tick(i64::MAX)`).
    pub fn saturating_div(self, rhs: i64) -> Option<Tick> {
        if rhs == 0 { None } else { Some(Tick(self.0.saturating_div(rhs))) }
    }
}

impl Add for Tick {
    type Output = Tick;
    fn add(self, o: Tick) -> Tick {
        Tick(self.0.saturating_add(o.0))
    }
}

impl Sub for Tick {
    type Output = Tick;
    fn sub(self, o: Tick) -> Tick {
        Tick(self.0.saturating_sub(o.0))
    }
}

impl Neg for Tick {
    type Output = Tick;
    fn neg(self) -> Tick {
        Tick(self.0.saturating_neg())
    }
}

impl AddAssign for Tick {
    fn add_assign(&mut self, o: Tick) {
        self.0 = self.0.saturating_add(o.0);
    }
}

impl SubAssign for Tick {
    fn sub_assign(&mut self, o: Tick) {
        self.0 = self.0.saturating_sub(o.0);
    }
}

impl Mul<i64> for Tick {
    type Output = Tick;
    fn mul(self, rhs: i64) -> Tick {
        Tick(self.0.saturating_mul(rhs))
    }
}

impl Div<i64> for Tick {
    type Output = Tick;
    fn div(self, rhs: i64) -> Tick {
        // Operator division policy: Div saturates on overflow (e.g. Tick(i64::MIN) / -1 -> Tick(i64::MAX))
        // using self.0.saturating_div(rhs) to preserve monotonicity across the full range.
        // Division by zero yields Tick::ZERO. For fallible division that detects zero divisors,
        // use Tick::checked_div().
        if rhs == 0 { Tick::ZERO } else { Tick(self.0.saturating_div(rhs)) }
    }
}

impl Rem<Tick> for Tick {
    type Output = Tick;
    fn rem(self, rhs: Tick) -> Tick {
        // Operator remainder policy: Rem produces exact remainder where defined,
        // and Tick::ZERO for zero divisor or Tick(i64::MIN) % Tick(-1) (which is mathematically 0).
        // For fallible remainder, use Tick::checked_rem().
        if rhs.0 == 0 || (self.0 == i64::MIN && rhs.0 == -1) { Tick::ZERO } else { Tick(self.0 % rhs.0) }
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ticks", self.0)
    }
}

/// A half-open time range `[start, start + duration)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Tick,
    pub duration: Tick,
}

impl TimeRange {
    pub fn new(start: Tick, duration: Tick) -> Self {
        Self { start, duration }
    }

    pub fn from_bounds(start: Tick, end: Tick) -> Self {
        Self { start, duration: end - start }
    }

    pub fn end(&self) -> Tick {
        self.start + self.duration
    }

    pub fn contains(&self, t: Tick) -> bool {
        t >= self.start && t < self.end()
    }

    pub fn overlaps(&self, o: &TimeRange) -> bool {
        !self.is_empty() && !o.is_empty() && self.start < o.end() && o.start < self.end()
    }

    pub fn intersect(&self, o: &TimeRange) -> Option<TimeRange> {
        let s = self.start.max(o.start);
        let e = self.end().min(o.end());
        (e > s).then(|| TimeRange::from_bounds(s, e))
    }

    pub fn is_empty(&self) -> bool {
        self.duration.0 <= 0
    }
}

/// A frame rate as an exact rational (`num / den` frames per second).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRate {
    pub num: i64,
    pub den: i64,
}

impl Default for FrameRate {
    fn default() -> Self {
        FrameRate::FPS_23_976
    }
}

impl FrameRate {
    pub const FPS_23_976: FrameRate = FrameRate { num: 24000, den: 1001 };
    pub const FPS_24: FrameRate = FrameRate { num: 24, den: 1 };
    pub const FPS_25: FrameRate = FrameRate { num: 25, den: 1 };
    pub const FPS_29_97: FrameRate = FrameRate { num: 30000, den: 1001 };
    pub const FPS_30: FrameRate = FrameRate { num: 30, den: 1 };
    pub const FPS_48: FrameRate = FrameRate { num: 48, den: 1 };
    pub const FPS_50: FrameRate = FrameRate { num: 50, den: 1 };
    pub const FPS_59_94: FrameRate = FrameRate { num: 60000, den: 1001 };
    pub const FPS_60: FrameRate = FrameRate { num: 60, den: 1 };
    pub const FPS_119_88: FrameRate = FrameRate { num: 120000, den: 1001 };
    pub const FPS_120: FrameRate = FrameRate { num: 120, den: 1 };

    /// Standard broadcast and cinematic rates.
    pub const COMMON: [FrameRate; 11] = [
        Self::FPS_23_976,
        Self::FPS_24,
        Self::FPS_25,
        Self::FPS_29_97,
        Self::FPS_30,
        Self::FPS_48,
        Self::FPS_50,
        Self::FPS_59_94,
        Self::FPS_60,
        Self::FPS_119_88,
        Self::FPS_120,
    ];

    pub fn new(num: i64, den: i64) -> Self {
        let g = i128::from(gcd(num.unsigned_abs(), den.unsigned_abs()).max(1));
        FrameRate { num: (i128::from(num) / g) as i64, den: (i128::from(den) / g) as i64 }
    }

    /// Closest standard rate for a floating-point frame rate.
    pub fn from_f64(fps: f64) -> Self {
        for r in Self::COMMON {
            if (r.as_f64() - fps).abs() < 0.005 {
                return r;
            }
        }
        FrameRate::new((fps * 1000.0).round() as i64, 1000)
    }

    /// Closest standard rate when `fps` is within `tolerance` of it, otherwise the nearest
    /// 1/1000 rate. This retains the timeline API's coarse-timebase compatibility behavior.
    pub fn from_f64_within(fps: f64, tolerance: f64) -> Self {
        let mut best: Option<(f64, FrameRate)> = None;
        for rate in Self::COMMON {
            let off = (rate.as_f64() - fps).abs();
            if off <= tolerance * rate.as_f64() && best.is_none_or(|(closest, _)| off < closest) {
                best = Some((off, rate));
            }
        }
        best.map_or_else(|| Self::from_f64(fps), |(_, rate)| rate)
    }

    /// Returns this frame rate if positive, or the default rate ([`FrameRate::FPS_23_976`]).
    ///
    /// ### Application Compatibility Note
    /// For non-positive or zero rates (e.g. `{0, 0}`), `craft-time` falls back to
    /// [`FrameRate::FPS_23_976`] (timecode base 24), providing a standard broadcast default.
    /// In contrast, EffectCraft falls back to timecode base 1.
    pub fn sane(self) -> FrameRate {
        if self.num > 0 && self.den > 0 { self } else { FrameRate::default() }
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Exact duration of one frame (floored for exotic rates).
    pub fn frame_duration(self) -> Tick {
        let r = self.sane();
        Tick(clamp_i64((TICKS_PER_SECOND as i128 * r.den as i128) / r.num as i128))
    }

    /// Index of the frame containing `t` (floor).
    pub fn frame_at(self, t: Tick) -> i64 {
        clamp_i64(frame_index(self.sane(), t))
    }

    /// Frames from `start` to `end`, counted between their frame indices.
    pub fn frames_between(self, start: Tick, end: Tick) -> i64 {
        self.frame_at(end).saturating_sub(self.frame_at(start))
    }

    /// Start tick of frame `f` (ceiling division).
    pub fn tick_of(self, f: i64) -> Tick {
        Tick(clamp_i64(frame_boundary(self.sane(), i128::from(f))))
    }

    /// Snap `t` down to a frame boundary.
    pub fn snap(self, t: Tick) -> Tick {
        let r = self.sane();
        Tick(clamp_i64(frame_boundary(r, frame_index(r, t))))
    }

    /// Snap `t` to the nearest frame boundary.
    pub fn snap_nearest(self, t: Tick) -> Tick {
        let r = self.sane();
        let f = frame_index(r, t);
        let a = frame_boundary(r, f);
        let b = frame_boundary(r, f.saturating_add(1));
        let t = i128::from(t.0);
        Tick(clamp_i64(if t - a <= b - t { a } else { b }))
    }

    /// Timecode base (nominal frames per second): 30 for 29.97, 24 for 23.976.
    pub fn timecode_base(self) -> i64 {
        let r = self.sane();
        (r.num / r.den).saturating_add(i64::from(r.num % r.den != 0)).max(1)
    }

    pub fn is_ntsc(self) -> bool {
        self.den == 1001
    }

    pub fn supports_drop_frame(self) -> bool {
        self.is_ntsc() && self.timecode_base() % 30 == 0
    }

    pub fn label(self) -> String {
        if self.den == 1 {
            format!("{}", self.num)
        } else {
            let v = self.as_f64();
            let s = format!("{v:.3}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
    }
}

impl fmt::Display for FrameRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} fps", self.label())
    }
}

fn clamp_i64(value: i128) -> i64 {
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

fn frame_index(rate: FrameRate, t: Tick) -> i128 {
    (i128::from(t.0) * i128::from(rate.num)).div_euclid(i128::from(TICKS_PER_SECOND) * i128::from(rate.den))
}

fn frame_boundary(rate: FrameRate, frame: i128) -> i128 {
    let Some(n) = frame.checked_mul(i128::from(TICKS_PER_SECOND)).and_then(|n| n.checked_mul(i128::from(rate.den))) else {
        return if frame < 0 { i128::MIN } else { i128::MAX };
    };
    let d = i128::from(rate.num);
    n.div_euclid(d) + i128::from(n.rem_euclid(d) != 0)
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
