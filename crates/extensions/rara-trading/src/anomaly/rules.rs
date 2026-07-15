// Copyright 2026 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! L1 window rules: cumulative return, rolling drawdown, and volume surge.
//! Each is a pure function over the ordered close/volume series so the
//! evaluator can compose them and every branch stays unit-testable.

/// Signed cumulative return across `closes`, as a fraction of the first close.
///
/// Returns `0.0` for a window of fewer than two prices or a non-positive first
/// close (the caller rejects non-positive prices before this point, so the
/// guard is defensive).
pub(crate) fn window_return(closes: &[f64]) -> f64 {
    match (closes.first(), closes.last()) {
        (Some(&first), Some(&last)) if first > 0.0 && closes.len() >= 2 => (last - first) / first,
        _ => 0.0,
    }
}

/// Deepest peak-to-trough decline across `closes`, as a positive fraction of
/// the running peak. `0.0` for a monotonically non-decreasing series.
pub(crate) fn max_drawdown(closes: &[f64]) -> f64 {
    let mut peak = f64::MIN;
    let mut worst = 0.0_f64;
    for &price in closes {
        peak = peak.max(price);
        if peak > 0.0 {
            worst = worst.max((peak - price) / peak);
        }
    }
    worst
}

/// Newest volume relative to the rolling mean of the historical volumes.
///
/// `None` when there is no history or the mean is ~zero (division would be
/// meaningless), so the caller treats the surge rule as not evaluable.
pub(crate) fn volume_surge(history_volumes: &[f64], latest_volume: f64) -> Option<f64> {
    if history_volumes.is_empty() {
        return None;
    }
    let mean = history_volumes.iter().sum::<f64>() / history_volumes.len() as f64;
    if mean <= f64::EPSILON {
        return None;
    }
    Some(latest_volume / mean)
}

#[cfg(test)]
mod tests {
    use super::{max_drawdown, volume_surge, window_return};

    #[test]
    fn window_return_is_signed_fraction_of_first_close() {
        let closes = [100.0, 101.0, 93.0];
        assert!((window_return(&closes) - (-0.07)).abs() < 1e-9);
        assert_eq!(window_return(&[100.0]), 0.0);
    }

    #[test]
    fn max_drawdown_tracks_peak_to_trough() {
        // Peak 110, trough 99 → drawdown 0.1.
        let closes = [100.0, 110.0, 99.0, 104.0];
        assert!((max_drawdown(&closes) - 0.1).abs() < 1e-9);
        // Monotonic rise has no drawdown.
        assert_eq!(max_drawdown(&[100.0, 101.0, 102.0]), 0.0);
    }

    #[test]
    fn volume_surge_is_ratio_to_rolling_mean() {
        let surge = volume_surge(&[100.0, 100.0, 100.0], 400.0).expect("non-empty history");
        assert!((surge - 4.0).abs() < 1e-9);
        assert!(volume_surge(&[], 400.0).is_none());
    }
}
