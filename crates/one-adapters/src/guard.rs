//! Proteção contra chamadas excessivas: serializa a computação e reaproveita o resultado por um
//! curto prazo. Quem chamar em laço executa as CLIs no máximo uma vez por `ttl`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Throttle<T: Clone> {
    ttl: Duration,
    slot: Mutex<Option<(Instant, bool, T)>>,
}

impl<T: Clone> Throttle<T> {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            slot: Mutex::new(None),
        }
    }

    /// Devolve o valor recente ou calcula um novo, com `f` rodando sob exclusão mútua.
    /// `full = true` exige um resultado "completo": um resultado parcial recente não serve.
    pub fn get_or_compute(&self, full: bool, f: impl FnOnce() -> T) -> T {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, was_full, v)) = &*slot {
            if at.elapsed() < self.ttl && (*was_full || !full) {
                return v.clone();
            }
        }
        let v = f();
        *slot = Some((Instant::now(), full, v.clone()));
        v
    }

    /// Descarta o resultado (ex.: após uma aprovação, que muda o que a detecção mostra).
    pub fn invalidate(&self) {
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn repeated_calls_within_ttl_compute_once_even_concurrently() {
        let t = Throttle::new(Duration::from_secs(60));
        let n = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    t.get_or_compute(true, || {
                        n.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(30));
                        7
                    })
                });
            }
        });
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn partial_result_does_not_satisfy_full_request_and_invalidate_works() {
        let t = Throttle::new(Duration::from_secs(60));
        assert_eq!(t.get_or_compute(false, || 1), 1);
        assert_eq!(t.get_or_compute(false, || 2), 1);
        assert_eq!(t.get_or_compute(true, || 3), 3);
        assert_eq!(t.get_or_compute(false, || 4), 3);
        t.invalidate();
        assert_eq!(t.get_or_compute(true, || 5), 5);
    }

    #[test]
    fn expires_after_ttl() {
        let t = Throttle::new(Duration::from_millis(20));
        assert_eq!(t.get_or_compute(true, || 1), 1);
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(t.get_or_compute(true, || 2), 2);
    }
}
