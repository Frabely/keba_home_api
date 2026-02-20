#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimestampMs(pub i64);

pub trait Clock {
    fn now(&self) -> TimestampMs;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTransition {
    Plugged {
        plugged_at: TimestampMs,
    },
    Unplugged {
        plugged_at: TimestampMs,
        unplugged_at: TimestampMs,
    },
}

#[derive(Debug, Clone)]
pub struct SessionStateMachine {
    debounce_samples: usize,
    stable_plugged: Option<bool>,
    candidate: Option<Candidate>,
    active_session_started_at: Option<TimestampMs>,
}

#[derive(Debug, Clone, Copy)]
struct Candidate {
    plugged: bool,
    count: usize,
}

impl SessionStateMachine {
    pub fn new(debounce_samples: usize) -> Self {
        Self {
            debounce_samples: debounce_samples.max(1),
            stable_plugged: None,
            candidate: None,
            active_session_started_at: None,
        }
    }

    pub fn observe<C: Clock>(
        &mut self,
        plugged_observation: bool,
        clock: &C,
    ) -> Option<SessionTransition> {
        match self.stable_plugged {
            None => {
                if self.accept_candidate(plugged_observation) {
                    self.stable_plugged = Some(plugged_observation);
                    self.candidate = None;
                }
                None
            }
            Some(stable) if stable == plugged_observation => {
                self.candidate = None;
                None
            }
            Some(stable) => {
                if !self.accept_candidate(plugged_observation) {
                    return None;
                }

                self.stable_plugged = Some(plugged_observation);
                self.candidate = None;

                if !stable && plugged_observation {
                    let plugged_at = clock.now();
                    self.active_session_started_at = Some(plugged_at);
                    return Some(SessionTransition::Plugged { plugged_at });
                }

                if stable && !plugged_observation {
                    let unplugged_at = clock.now();
                    let plugged_at = self
                        .active_session_started_at
                        .take()
                        .unwrap_or(unplugged_at);

                    return Some(SessionTransition::Unplugged {
                        plugged_at,
                        unplugged_at,
                    });
                }

                None
            }
        }
    }

    pub fn active_session_started_at(&self) -> Option<TimestampMs> {
        self.active_session_started_at
    }

    fn accept_candidate(&mut self, plugged_observation: bool) -> bool {
        match self.candidate {
            Some(mut candidate) if candidate.plugged == plugged_observation => {
                candidate.count += 1;
                self.candidate = Some(candidate);
                candidate.count >= self.debounce_samples
            }
            _ => {
                self.candidate = Some(Candidate {
                    plugged: plugged_observation,
                    count: 1,
                });
                self.debounce_samples == 1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{Clock, SessionStateMachine, SessionTransition, TimestampMs};

    struct FakeClock {
        now: Cell<i64>,
    }

    impl FakeClock {
        fn new(start: i64) -> Self {
            Self {
                now: Cell::new(start),
            }
        }

        fn set(&self, value: i64) {
            self.now.set(value);
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> TimestampMs {
            TimestampMs(self.now.get())
        }
    }

    #[test]
    fn emits_plugged_after_debounce_threshold() {
        let clock = FakeClock::new(1_000);
        let mut machine = SessionStateMachine::new(2);

        assert_eq!(machine.observe(false, &clock), None);
        assert_eq!(machine.observe(false, &clock), None);

        clock.set(2_000);
        assert_eq!(machine.observe(true, &clock), None);
        assert_eq!(
            machine.observe(true, &clock),
            Some(SessionTransition::Plugged {
                plugged_at: TimestampMs(2_000),
            })
        );
    }

    #[test]
    fn emits_unplugged_with_session_bounds() {
        let clock = FakeClock::new(1_000);
        let mut machine = SessionStateMachine::new(2);

        machine.observe(false, &clock);
        machine.observe(false, &clock);

        clock.set(2_000);
        machine.observe(true, &clock);
        machine.observe(true, &clock);

        clock.set(5_000);
        assert_eq!(machine.observe(false, &clock), None);
        assert_eq!(
            machine.observe(false, &clock),
            Some(SessionTransition::Unplugged {
                plugged_at: TimestampMs(2_000),
                unplugged_at: TimestampMs(5_000),
            })
        );
        assert_eq!(machine.active_session_started_at(), None);
    }

    #[test]
    fn startup_in_plugged_state_does_not_emit_transition() {
        let clock = FakeClock::new(1_000);
        let mut machine = SessionStateMachine::new(2);

        assert_eq!(machine.observe(true, &clock), None);
        assert_eq!(machine.observe(true, &clock), None);
        assert_eq!(machine.active_session_started_at(), None);
    }

    #[test]
    fn flap_does_not_trigger_transition() {
        let clock = FakeClock::new(1_000);
        let mut machine = SessionStateMachine::new(2);

        machine.observe(false, &clock);
        machine.observe(false, &clock);

        assert_eq!(machine.observe(true, &clock), None);
        assert_eq!(machine.observe(false, &clock), None);
        assert_eq!(machine.observe(true, &clock), None);
        assert_eq!(machine.observe(false, &clock), None);
    }
}
