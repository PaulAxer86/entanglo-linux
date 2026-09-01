//! RTT trend + packet-loss estimate from heartbeat sequence gaps,
//! feeding the Network Quality dashboard tile. Ported from the real
//! `entanglo-macos` `NetworkQualityService.suggestion()` (read
//! directly from its Swift source, not guessed): offline if no
//! heartbeat has ever arrived or the last one is stale (> 3 s);
//! unstable if avg RTT ≥ 120 ms or loss ≥ 5%; otherwise a preference
//! based on interface kind.
//!
//! **Known gap vs. the Mac**: interface kind (Ethernet vs. Wi-Fi)
//! isn't tracked per-connection on the Linux side yet — no code path
//! here distinguishes them, unlike `entanglo-macos`'s
//! `NetworkTransport.interfaceKind` (sampled from `NWPath`). When not
//! `Unstable`/`Offline`, this always returns `WifiOk`, matching the
//! Mac's own conservative default for its `.unknown` interface case.
//! First version of this module (before this correction) used
//! invented thresholds (<5 ms / <30 ms) that didn't match the real
//! Mac at all — found by actually reading `NetworkQualityService.swift`
//! after the Network page showed a live RTT the old thresholds
//! mis-classified.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestedMode {
    EthernetPreferred,
    WifiOk,
    Unstable,
    /// No heartbeat has ever arrived, or the last one is stale.
    Offline,
}

/// Real Mac constants (`NetworkQualityService.swift`), not invented.
const UNSTABLE_LATENCY_MS: f64 = 120.0;
const UNSTABLE_LOSS_RATE: f64 = 0.05;
const HEARTBEAT_STALE_AFTER: Duration = Duration::from_secs(3);

pub struct NetworkQualityMonitor {
    rtt_samples_ms: Vec<f64>,
    last_sequence_seen: Option<u64>,
    missed_heartbeats: u64,
    /// Total heartbeats we'd expect to have seen by `last_sequence_seen`
    /// (i.e. the highest sequence number observed) — the denominator
    /// for the loss-rate estimate. `missed_heartbeats / this` mirrors
    /// what the Mac calls `lossRate`, though its own computation is
    /// Mac-internal and not ported byte-for-byte — the real source
    /// doesn't expose how `lossRate` itself is computed, only the
    /// threshold it's compared against.
    total_expected: u64,
    last_heartbeat_at: Option<Instant>,
}

impl NetworkQualityMonitor {
    pub fn new() -> Self {
        Self {
            rtt_samples_ms: Vec::new(),
            last_sequence_seen: None,
            missed_heartbeats: 0,
            total_expected: 0,
            last_heartbeat_at: None,
        }
    }

    pub fn record_heartbeat(&mut self, sequence: u64, rtt_ms: Option<f64>) {
        self.last_heartbeat_at = Some(Instant::now());
        if let Some(rtt) = rtt_ms {
            self.rtt_samples_ms.push(rtt);
            if self.rtt_samples_ms.len() > 60 {
                self.rtt_samples_ms.remove(0);
            }
        }
        if let Some(last) = self.last_sequence_seen {
            let gap = sequence.saturating_sub(last).saturating_sub(1);
            self.missed_heartbeats += gap;
        }
        self.last_sequence_seen = Some(sequence);
        self.total_expected = self.total_expected.max(sequence + 1);
    }

    pub fn average_rtt_ms(&self) -> Option<f64> {
        if self.rtt_samples_ms.is_empty() {
            return None;
        }
        Some(self.rtt_samples_ms.iter().sum::<f64>() / self.rtt_samples_ms.len() as f64)
    }

    pub fn loss_rate(&self) -> Option<f64> {
        if self.total_expected == 0 {
            return None;
        }
        Some(self.missed_heartbeats as f64 / self.total_expected as f64)
    }

    pub fn suggested_mode(&self) -> SuggestedMode {
        let Some(last_heartbeat_at) = self.last_heartbeat_at else {
            return SuggestedMode::Offline;
        };
        if last_heartbeat_at.elapsed() > HEARTBEAT_STALE_AFTER {
            return SuggestedMode::Offline;
        }

        let latency = self.average_rtt_ms().unwrap_or(0.0);
        let loss = self.loss_rate().unwrap_or(0.0);
        if latency >= UNSTABLE_LATENCY_MS || loss >= UNSTABLE_LOSS_RATE {
            return SuggestedMode::Unstable;
        }

        // No per-connection interface-kind tracking on Linux yet — see
        // the module doc comment.
        SuggestedMode::WifiOk
    }
}

impl Default for NetworkQualityMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_before_any_heartbeat() {
        let monitor = NetworkQualityMonitor::new();
        assert_eq!(monitor.suggested_mode(), SuggestedMode::Offline);
    }

    #[test]
    fn wifi_ok_under_the_real_120ms_threshold() {
        let mut monitor = NetworkQualityMonitor::new();
        monitor.record_heartbeat(0, Some(50.0));
        assert_eq!(monitor.suggested_mode(), SuggestedMode::WifiOk);
    }

    #[test]
    fn unstable_at_or_above_the_real_120ms_threshold() {
        let mut monitor = NetworkQualityMonitor::new();
        monitor.record_heartbeat(0, Some(120.0));
        assert_eq!(monitor.suggested_mode(), SuggestedMode::Unstable);

        let mut monitor = NetworkQualityMonitor::new();
        monitor.record_heartbeat(0, Some(998.4));
        assert_eq!(
            monitor.suggested_mode(),
            SuggestedMode::Unstable,
            "matches what was actually observed live against the real Mac"
        );
    }

    #[test]
    fn loss_rate_reflects_sequence_gaps() {
        let mut monitor = NetworkQualityMonitor::new();
        monitor.record_heartbeat(0, Some(10.0));
        monitor.record_heartbeat(1, Some(10.0));
        monitor.record_heartbeat(4, Some(10.0)); // sequences 2,3 missed
        assert_eq!(monitor.loss_rate(), Some(2.0 / 5.0));
    }
}
