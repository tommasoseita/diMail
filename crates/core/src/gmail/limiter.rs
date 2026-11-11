//! Limitatore delle unità di quota di Gmail.
//!
//! Dal 1° maggio 2026 i progetti nuovi hanno 6.000 unità al minuto per utente, e `threads.get`
//! completo ne costa 120. Il limitatore tiene il conto di quanto speso negli ultimi 60 secondi, in
//! finestra mobile, e non supera un budget del 90% del limite (5.400). Una raffica all'avvio è
//! permessa: è quello che fa comparire l'inbox in pochi secondi.
//!
//! La sync storica non usa più dell'85% del budget, così le azioni dell'utente e i thread aperti
//! trovano sempre quota libera (circa 800 unità al minuto: una decina di thread aperti senza corpo),
//! e cede il passo appena una richiesta urgente aspetta.
//!
//! Se Gmail rifiuta lo stesso (il conto di Google include richieste che noi non vediamo, per esempio
//! di un'istanza appena chiusa), il budget scende del 40% e tutti si fermano: 5 s, poi il doppio se i
//! rifiuti continuano, o l'attesa indicata da Gmail. Ogni richiesta andata bene lo fa risalire.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Azioni dell'utente, thread aperti, sync incrementale.
    Interactive,
    /// Prima sync storica.
    Background,
}

/// Limite di Gmail per utente e progetto, in unità al minuto.
pub const GMAIL_UNITS_PER_MINUTE: u32 = 6_000;
/// Il nostro budget: 90% del limite.
pub const UNITS_PER_MINUTE: u32 = GMAIL_UNITS_PER_MINUTE * 9 / 10;
const WINDOW: Duration = Duration::from_secs(60);
const BACKGROUND_SHARE: f64 = 0.85;
const MIN_BUDGET: f64 = 1_500.0;
const RECOVERY: f64 = 100.0;
const FIRST_BACKOFF: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(80);
/// Tetto all'attesa chiesta da Gmail, contro valori assurdi.
const MAX_HINT: Duration = Duration::from_secs(30 * 60);

struct State {
    /// Unità spese negli ultimi 60 secondi.
    spent: VecDeque<(Instant, u32)>,
    spent_sum: u32,
    budget: f64,
    max_budget: f64,
    paused_until: Option<Instant>,
    /// Rifiuti di fila senza una richiesta andata bene in mezzo.
    strikes: u32,
}

impl State {
    fn prune(&mut self, now: Instant) {
        while let Some(&(t, u)) = self.spent.front() {
            if now.saturating_duration_since(t) < WINDOW {
                break;
            }
            self.spent.pop_front();
            self.spent_sum -= u;
        }
    }

    /// Quanto aspettare perché `units` stiano sotto `limit`.
    fn wait_for(&self, units: u32, limit: f64, now: Instant) -> Duration {
        if f64::from(self.spent_sum + units) <= limit || self.spent.is_empty() {
            // Una richiesta più grande del budget intero parte a finestra vuota.
            return Duration::ZERO;
        }
        let mut sum = self.spent_sum;
        for &(t, u) in &self.spent {
            sum -= u;
            if f64::from(sum + units) <= limit {
                return (t + WINDOW).saturating_duration_since(now);
            }
        }
        self.spent.back().map(|&(t, _)| (t + WINDOW).saturating_duration_since(now)).unwrap_or_default()
    }
}

pub struct Limiter {
    state: Mutex<State>,
    interactive_waiting: AtomicUsize,
}

struct WaitingGuard<'a>(&'a AtomicUsize);
impl Drop for WaitingGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Limiter {
    pub fn new(units_per_minute: u32) -> Self {
        Limiter {
            state: Mutex::new(State {
                spent: VecDeque::new(),
                spent_sum: 0,
                budget: f64::from(units_per_minute),
                max_budget: f64::from(units_per_minute),
                paused_until: None,
                strikes: 0,
            }),
            interactive_waiting: AtomicUsize::new(0),
        }
    }

    pub async fn acquire(&self, units: u32, prio: Priority) {
        let _guard = (prio == Priority::Interactive).then(|| {
            self.interactive_waiting.fetch_add(1, Ordering::SeqCst);
            WaitingGuard(&self.interactive_waiting)
        });
        loop {
            let wait = {
                let mut s = self.state.lock().await;
                let now = Instant::now();
                s.prune(now);
                let limit = match prio {
                    Priority::Interactive => s.budget,
                    Priority::Background => s.budget * BACKGROUND_SHARE,
                };
                let mut wait = s.paused_until.map(|p| p.saturating_duration_since(now)).unwrap_or_default();
                wait = wait.max(s.wait_for(units, limit, now));
                if prio == Priority::Background && self.interactive_waiting.load(Ordering::SeqCst) > 0 {
                    wait = wait.max(Duration::from_millis(50));
                }
                if wait.is_zero() {
                    s.spent.push_back((now, units));
                    s.spent_sum += units;
                    return;
                }
                // Si ricontrolla almeno ogni secondo: una richiesta urgente può essere arrivata.
                wait.min(Duration::from_secs(1))
            };
            tokio::time::sleep(wait).await;
        }
    }

    /// Gmail ha rifiutato per quota: abbassa il budget e ferma tutti. `hint` è l'attesa indicata
    /// da Gmail, se c'era. Restituisce quanto dura la pausa.
    ///
    /// I rifiuti che arrivano mentre la pausa è già in corso non contano di nuovo: sono richieste
    /// partite prima, magari dall'altra corsia.
    pub async fn throttled(&self, hint: Option<Duration>) -> Duration {
        let mut s = self.state.lock().await;
        let now = Instant::now();
        let hint = hint.map(|h| h.min(MAX_HINT));
        if let Some(p) = s.paused_until.filter(|p| *p > now) {
            if let Some(h) = hint.filter(|h| now + *h > p) {
                s.paused_until = Some(now + h);
                return h;
            }
            return p - now;
        }
        s.strikes += 1;
        s.budget = (s.budget * 0.6).max(MIN_BUDGET);
        let backoff = FIRST_BACKOFF.saturating_mul(1 << (s.strikes - 1).min(5)).min(MAX_BACKOFF);
        let wait = hint.unwrap_or(backoff);
        s.paused_until = Some(now + wait);
        wait
    }

    /// Una richiesta è andata bene: il budget risale un poco.
    pub async fn succeeded(&self) {
        let mut s = self.state.lock().await;
        s.strikes = 0;
        s.budget = (s.budget + RECOVERY).min(s.max_budget);
    }

    /// Budget corrente e unità spese nell'ultimo minuto, per i log.
    pub async fn usage(&self) -> (u32, u32) {
        let mut s = self.state.lock().await;
        s.prune(Instant::now());
        (s.budget as u32, s.spent_sum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn burst_then_waits_for_the_window_to_slide() {
        let l = Limiter::new(1_000);
        let t0 = Instant::now();
        for _ in 0..10 {
            l.acquire(100, Priority::Interactive).await; // 1.000 unità subito
        }
        assert!(t0.elapsed() < Duration::from_millis(5));
        l.acquire(100, Priority::Interactive).await; // aspetta che esca la prima
        let e = t0.elapsed();
        assert!(e >= Duration::from_secs(60) && e <= Duration::from_millis(61_100), "{e:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn background_leaves_room_for_the_user() {
        let l = Limiter::new(1_000);
        let t0 = Instant::now();
        for _ in 0..8 {
            l.acquire(100, Priority::Background).await; // 80% del budget
        }
        l.acquire(50, Priority::Background).await; // 85%
        assert!(t0.elapsed() < Duration::from_millis(5));
        // Lo storico è fermo, l'utente no.
        l.acquire(150, Priority::Interactive).await;
        assert!(t0.elapsed() < Duration::from_millis(5));
        l.acquire(100, Priority::Background).await;
        assert!(t0.elapsed() >= Duration::from_secs(60));
    }

    #[tokio::test(start_paused = true)]
    async fn throttle_pauses_everyone_and_lowers_the_budget() {
        let l = Limiter::new(5_000);
        assert_eq!(l.throttled(None).await, FIRST_BACKOFF);
        assert_eq!(l.usage().await.0, 3_000);
        // Un secondo rifiuto durante la pausa non conta di nuovo.
        assert!(l.throttled(None).await <= FIRST_BACKOFF);
        assert_eq!(l.usage().await.0, 3_000);

        let t0 = Instant::now();
        l.acquire(1, Priority::Interactive).await;
        assert!(t0.elapsed() >= Duration::from_millis(4_990));

        // Rifiuti di fila dopo la pausa: l'attesa raddoppia, il budget non scende sotto il minimo.
        assert_eq!(l.throttled(None).await, FIRST_BACKOFF * 2);
        for _ in 0..10 {
            tokio::time::advance(MAX_BACKOFF).await;
            l.throttled(None).await;
        }
        assert_eq!(l.usage().await.0, MIN_BUDGET as u32);
        tokio::time::advance(MAX_BACKOFF).await;
        assert_eq!(l.throttled(Some(Duration::from_secs(7))).await, Duration::from_secs(7));
    }

    #[tokio::test(start_paused = true)]
    async fn budget_recovers_after_successes() {
        let l = Limiter::new(5_000);
        l.throttled(None).await;
        assert_eq!(l.usage().await.0, 3_000);
        for _ in 0..30 {
            l.succeeded().await;
        }
        assert_eq!(l.usage().await.0, 5_000);
        // Dopo un successo i rifiuti ripartono dalla prima attesa.
        tokio::time::advance(FIRST_BACKOFF).await;
        assert_eq!(l.throttled(None).await, FIRST_BACKOFF);
    }
}
