//! How often one address may ask to decline.

use std::collections::HashMap;
use std::net::IpAddr;

/// How many refusals one address may ask for within one window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limit {
    /// How many asks are let through in a window.
    pub(crate) asks: usize,
    /// How long a window lasts, in seconds.
    pub(crate) window: u64,
}

impl Limit {
    /// Five a minute: a person declines once, and a 128-bit token is not guessed at
    /// any rate, so this blunts noise rather than an attack that could succeed.
    pub(crate) const fn standard() -> Self {
        Self {
            asks: 5,
            window: 60,
        }
    }
}

/// The asks each address made in its current window.
#[derive(Debug)]
pub(crate) struct Limiter {
    limit: Limit,
    asked: HashMap<IpAddr, Vec<u64>>,
}

impl Limiter {
    /// A limiter holding to `limit`.
    pub(crate) fn new(limit: Limit) -> Self {
        Self {
            limit,
            asked: HashMap::new(),
        }
    }

    /// Whether `from` may ask at `now`, in seconds; an ask let through is counted.
    pub(crate) fn admits(&mut self, from: IpAddr, now: u64) -> bool {
        let window = self.limit.window;
        self.asked
            .retain(|_, times| times.iter().any(|at| now.saturating_sub(*at) < window));
        let times = self.asked.entry(from).or_default();
        times.retain(|at| now.saturating_sub(*at) < window);
        if times.len() >= self.limit.asks {
            return false;
        }
        times.push(now);
        true
    }
}

#[cfg(test)]
mod tests;
