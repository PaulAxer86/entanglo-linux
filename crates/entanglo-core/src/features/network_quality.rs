//! RTT trend + packet-loss estimate from heartbeat sequence gaps,
//! feeding the Network Quality dashboard tile. Phase 2, see
//! `ROADMAP.md`. Algorithm mirrors
//! `entanglo-macos`'s `NetworkQualityService`/`NetworkQualityMonitor`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestedMode {
    EthernetPreferred,
    WifiOk,
    Unstable,
}

pub struct NetworkQualityMonitor {
    rtt_samples_ms: Vec<f64>,
    last_sequence_seen: Option<u64>,
    missed_heartbeats: u32,
}

impl NetworkQualityMonitor {
    pub fn new() -> Self {
        Self {
            rtt_samples_ms: Vec::new(),
            last_sequence_seen: None,
            missed_heartbeats: 0,
        }
    }

    pub fn record_heartbeat(&mut self, sequence: u64, rtt_ms: Option<f64>) {
        if let Some(rtt) = rtt_ms {
            self.rtt_samples_ms.push(rtt);
            if self.rtt_samples_ms.len() > 60 {
                self.rtt_samples_ms.remove(0);
            }
        }
        if let Some(last) = self.last_sequence_seen {
            let gap = sequence.saturating_sub(last).saturating_sub(1);
            self.missed_heartbeats += gap as u32;
        }
        self.last_sequence_seen = Some(sequence);
    }

    pub fn average_rtt_ms(&self) -> Option<f64> {
        if self.rtt_samples_ms.is_empty() {
            return None;
        }
        Some(self.rtt_samples_ms.iter().sum::<f64>() / self.rtt_samples_ms.len() as f64)
    }

    pub fn suggested_mode(&self) -> SuggestedMode {
        match self.average_rtt_ms() {
            Some(rtt) if rtt < 5.0 && self.missed_heartbeats == 0 => {
                SuggestedMode::EthernetPreferred
            }
            Some(rtt) if rtt < 30.0 => SuggestedMode::WifiOk,
            _ => SuggestedMode::Unstable,
        }
    }
}

impl Default for NetworkQualityMonitor {
    fn default() -> Self {
        Self::new()
    }
}
