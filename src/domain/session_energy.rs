use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnergySnapshot {
    pub present_session_kwh: Option<f64>,
    pub total_kwh: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionEnergyResult {
    pub kwh: f64,
    pub source: EnergySource,
    pub warnings: Vec<EnergyWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnergySource {
    PresentSession,
    PresentSessionDelta,
    TotalDelta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnergyWarning {
    NegativePresentSessionValueClamped,
    NegativePresentSessionDeltaClamped,
    NegativeTotalDeltaClamped,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum EnergyComputationError {
    #[error("unable to compute session energy: no usable energy data")]
    NoUsableEnergyData,
}

pub fn compute_session_kwh(
    start: Option<&EnergySnapshot>,
    end: &EnergySnapshot,
) -> Result<SessionEnergyResult, EnergyComputationError> {
    if let Some(end_present) = end.present_session_kwh {
        if let Some(start_present) = start.and_then(|snapshot| snapshot.present_session_kwh) {
            let raw_delta = end_present - start_present;
            if raw_delta < 0.0 {
                return Ok(SessionEnergyResult {
                    kwh: 0.0,
                    source: EnergySource::PresentSessionDelta,
                    warnings: vec![EnergyWarning::NegativePresentSessionDeltaClamped],
                });
            }

            return Ok(SessionEnergyResult {
                kwh: raw_delta,
                source: EnergySource::PresentSessionDelta,
                warnings: Vec::new(),
            });
        }

        if end_present < 0.0 {
            return Ok(SessionEnergyResult {
                kwh: 0.0,
                source: EnergySource::PresentSession,
                warnings: vec![EnergyWarning::NegativePresentSessionValueClamped],
            });
        }

        return Ok(SessionEnergyResult {
            kwh: end_present,
            source: EnergySource::PresentSession,
            warnings: Vec::new(),
        });
    }

    if let (Some(start_total), Some(end_total)) =
        (start.and_then(|snapshot| snapshot.total_kwh), end.total_kwh)
    {
        let raw_delta = end_total - start_total;
        if raw_delta < 0.0 {
            return Ok(SessionEnergyResult {
                kwh: 0.0,
                source: EnergySource::TotalDelta,
                warnings: vec![EnergyWarning::NegativeTotalDeltaClamped],
            });
        }

        return Ok(SessionEnergyResult {
            kwh: raw_delta,
            source: EnergySource::TotalDelta,
            warnings: Vec::new(),
        });
    }

    Err(EnergyComputationError::NoUsableEnergyData)
}

#[cfg(test)]
mod tests {
    use super::{
        EnergyComputationError, EnergySnapshot, EnergySource, EnergyWarning, compute_session_kwh,
    };

    #[test]
    fn uses_present_session_delta_when_available() {
        let start = EnergySnapshot {
            present_session_kwh: Some(1.2),
            total_kwh: Some(200.0),
        };
        let end = EnergySnapshot {
            present_session_kwh: Some(10.8),
            total_kwh: Some(210.0),
        };

        let result = compute_session_kwh(Some(&start), &end).expect("computation must succeed");

        assert_eq!(result.source, EnergySource::PresentSessionDelta);
        assert!((result.kwh - 9.6).abs() < 1e-9);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn clamps_negative_present_session_delta() {
        let start = EnergySnapshot {
            present_session_kwh: Some(10.0),
            total_kwh: None,
        };
        let end = EnergySnapshot {
            present_session_kwh: Some(3.0),
            total_kwh: None,
        };

        let result = compute_session_kwh(Some(&start), &end).expect("computation must succeed");

        assert_eq!(result.source, EnergySource::PresentSessionDelta);
        assert_eq!(result.kwh, 0.0);
        assert_eq!(
            result.warnings,
            vec![EnergyWarning::NegativePresentSessionDeltaClamped]
        );
    }

    #[test]
    fn uses_present_session_absolute_when_start_missing() {
        let end = EnergySnapshot {
            present_session_kwh: Some(8.4),
            total_kwh: Some(110.0),
        };

        let result = compute_session_kwh(None, &end).expect("computation must succeed");

        assert_eq!(result.source, EnergySource::PresentSession);
        assert!((result.kwh - 8.4).abs() < 1e-9);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn falls_back_to_total_delta_when_present_missing() {
        let start = EnergySnapshot {
            present_session_kwh: None,
            total_kwh: Some(100.5),
        };
        let end = EnergySnapshot {
            present_session_kwh: None,
            total_kwh: Some(110.5),
        };

        let result = compute_session_kwh(Some(&start), &end).expect("computation must succeed");

        assert_eq!(result.source, EnergySource::TotalDelta);
        assert!((result.kwh - 10.0).abs() < 1e-9);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn clamps_negative_total_delta() {
        let start = EnergySnapshot {
            present_session_kwh: None,
            total_kwh: Some(120.0),
        };
        let end = EnergySnapshot {
            present_session_kwh: None,
            total_kwh: Some(110.0),
        };

        let result = compute_session_kwh(Some(&start), &end).expect("computation must succeed");

        assert_eq!(result.source, EnergySource::TotalDelta);
        assert_eq!(result.kwh, 0.0);
        assert_eq!(
            result.warnings,
            vec![EnergyWarning::NegativeTotalDeltaClamped]
        );
    }

    #[test]
    fn fails_when_no_usable_energy_data_exists() {
        let start = EnergySnapshot {
            present_session_kwh: None,
            total_kwh: None,
        };
        let end = EnergySnapshot {
            present_session_kwh: None,
            total_kwh: None,
        };

        let result = compute_session_kwh(Some(&start), &end);

        assert_eq!(result, Err(EnergyComputationError::NoUsableEnergyData));
    }
}
