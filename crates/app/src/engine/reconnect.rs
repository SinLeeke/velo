//! Presupuesto por ventana temporal, sin hilo ni timer periódico.
//! Adaptado de Spotifast (https://github.com/crmne/spotifast, MIT, © 2026 Carmine Paolino)
//! src/backend.rs:50,62,2572 (commit 12deee4). Velo conserva su backoff 2/10/30 s.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const WINDOW: Duration = Duration::from_secs(600);
const LIMIT: usize = 6;

#[derive(Default)]
pub(super) struct ReconnectBudget {
    drops: VecDeque<Instant>,
}

impl ReconnectBudget {
    /// Cuenta un ciclo de reconexión, aunque consiga instalar otra sesión. Una sesión
    /// que nace y cae enseguida no reinicia el presupuesto y no produce un bucle infinito.
    pub fn record(&mut self, now: Instant) -> bool {
        while self
            .drops
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= WINDOW)
        {
            self.drops.pop_front();
        }
        if self.drops.len() >= LIMIT {
            return false;
        }
        self.drops.push_back(now);
        true
    }

    pub fn reset(&mut self) {
        self.drops.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_successful_reconnections_still_exhaust_the_window() {
        let mut budget = ReconnectBudget::default();
        let start = Instant::now();
        for attempt in 0..LIMIT {
            assert!(budget.record(start + Duration::from_secs(attempt as u64)));
        }
        assert!(!budget.record(start + Duration::from_secs(100)));
        // En el borde exacto vence solo la primera caída, no la ventana completa.
        assert!(budget.record(start + WINDOW));
        assert!(!budget.record(start + WINDOW));
        assert!(budget.record(start + WINDOW + Duration::from_secs(1)));
        assert_eq!(budget.drops.len(), LIMIT);
    }

    #[test]
    fn explicit_reset_and_expiry_allow_recovery_without_a_timer() {
        let mut budget = ReconnectBudget::default();
        let start = Instant::now();
        for _ in 0..LIMIT {
            assert!(budget.record(start));
        }
        assert!(!budget.record(start));
        budget.reset();
        assert!(budget.record(start));
        assert!(budget.record(start + WINDOW));
        assert_eq!(budget.drops.len(), 1);
    }
}
