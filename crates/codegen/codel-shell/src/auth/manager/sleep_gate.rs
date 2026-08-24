use std::time::{Duration, Instant, SystemTime};

pub const DARK_WAKE_DEFER_MAX: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
pub struct SleepGate;

impl SleepGate {
    pub fn raise(&self) -> GateRaise {
        GateRaise::now()
    }
}

#[derive(Debug)]
pub struct GateRaise {
    raised_at: Instant,
    pub wall: SystemTime,
}

impl GateRaise {
    pub fn now() -> Self {
        Self { raised_at: Instant::now(), wall: SystemTime::now() }
    }

    pub fn elapsed(&self) -> (Duration, Duration) {
        let mono = self.raised_at.elapsed();
        let wall = self.wall.elapsed().unwrap_or_default();
        (mono, wall)
    }
}

pub struct InFlightGuard;

impl InFlightGuard {
    pub fn new(_manager: &super::AuthManager) -> Self {
        Self
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {}
}
