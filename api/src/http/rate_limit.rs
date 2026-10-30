//! Limitation de débit par adresse cliente, pour les routes ouvertes sans authentification
//! (connexion, enrôlement, OAuth) : freine la force brute, le verrouillage de comptes par un
//! tiers et l'inondation du journal d'audit par des refus.
//!
//! Fenêtre fixe par adresse, en mémoire : chaque instance de l'API compte pour elle-même.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::error::{ErrorCode, Problem};
use super::policy::{ClientIp, HttpPolicy};

/// Au-delà, les fenêtres échues sont oubliées avant d'en ouvrir une nouvelle.
const PURGE_ABOVE: usize = 4096;

#[derive(Debug)]
pub struct RateLimiter {
    limit: u32,
    window: Duration,
    windows: Mutex<HashMap<IpAddr, Window>>,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    opened: Instant,
    count: u32,
}

impl RateLimiter {
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Compte une requête ; `Err(attente)` quand la fenêtre de l'adresse est pleine.
    pub fn check(&self, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        let mut windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        if windows.len() >= PURGE_ABOVE && !windows.contains_key(&ip) {
            let window = self.window;
            windows.retain(|_, w| now.duration_since(w.opened) < window);
        }
        let window = windows.entry(ip).or_insert(Window {
            opened: now,
            count: 0,
        });
        if now.duration_since(window.opened) >= self.window {
            *window = Window {
                opened: now,
                count: 0,
            };
        }
        if window.count >= self.limit {
            return Err(self.window - now.duration_since(window.opened));
        }
        window.count += 1;
        Ok(())
    }
}

/// Couche de route : `429 rate_limited` (avec `Retry-After`) quand l'adresse a épuisé sa
/// fenêtre. Sans adresse établie ni politique (routeur monté sans [`super::policy::apply`]),
/// la requête passe.
pub async fn limit_unauthenticated(request: Request, next: Next) -> Response {
    let limiter = request.extensions().get::<HttpPolicy>().cloned();
    let ip = request.extensions().get::<ClientIp>().copied();
    if let (Some(policy), Some(ClientIp(ip))) = (limiter, ip)
        && let Some(limiter) = policy.limiter()
        && let Err(retry_after) = limiter.check(ip, Instant::now())
    {
        return rejected(retry_after);
    }
    next.run(request).await
}

fn rejected(retry_after: Duration) -> Response {
    let seconds = retry_after.as_secs().max(1);
    let mut response = Problem::new(StatusCode::TOO_MANY_REQUESTS, ErrorCode::RateLimited)
        .param("retry_after_secs", seconds)
        .into_response();
    if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn each_address_has_its_own_window() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        let now = Instant::now();
        assert!(limiter.check(ip("203.0.113.1"), now).is_ok());
        assert!(limiter.check(ip("203.0.113.1"), now).is_ok());
        let wait = limiter.check(ip("203.0.113.1"), now).unwrap_err();
        assert_eq!(wait, Duration::from_secs(60));
        assert!(limiter.check(ip("203.0.113.2"), now).is_ok());
    }

    #[test]
    fn a_window_reopens_once_elapsed() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let now = Instant::now();
        assert!(limiter.check(ip("203.0.113.1"), now).is_ok());
        let wait = limiter
            .check(ip("203.0.113.1"), now + Duration::from_secs(15))
            .unwrap_err();
        assert_eq!(wait, Duration::from_secs(45));
        assert!(
            limiter
                .check(ip("203.0.113.1"), now + Duration::from_secs(60))
                .is_ok()
        );
    }

    #[test]
    fn elapsed_windows_are_purged_when_the_table_grows() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let now = Instant::now();
        for i in 0..PURGE_ABOVE {
            let octets = u32::try_from(i).unwrap().to_be_bytes();
            let ip = IpAddr::from([10, octets[1], octets[2], octets[3]]);
            assert!(limiter.check(ip, now).is_ok());
        }
        assert_eq!(limiter.windows.lock().unwrap().len(), PURGE_ABOVE);
        assert!(
            limiter
                .check(ip("203.0.113.1"), now + Duration::from_secs(61))
                .is_ok()
        );
        assert_eq!(limiter.windows.lock().unwrap().len(), 1);
    }
}
