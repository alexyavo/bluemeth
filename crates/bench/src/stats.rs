//! Tiny descriptive statistics, enough for a benchmark table.

/// Summary of a sample set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub n: usize,
    pub mean: f64,
    pub stdev: f64,
    pub min: f64,
    pub median: f64,
    pub p95: f64,
    pub max: f64,
}

impl Summary {
    /// Summarise `values`. Returns all-zero for an empty slice.
    pub fn of(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self {
                n: 0,
                mean: 0.0,
                stdev: 0.0,
                min: 0.0,
                median: 0.0,
                p95: 0.0,
                max: 0.0,
            };
        }
        let mut sorted: Vec<f64> = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let n = sorted.len();
        let mean = sorted.iter().sum::<f64>() / n as f64;
        let var = sorted.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64;
        Self {
            n,
            mean,
            stdev: var.sqrt(),
            min: sorted[0],
            median: percentile(&sorted, 50.0),
            p95: percentile(&sorted, 95.0),
            max: sorted[n - 1],
        }
    }
}

/// Nearest-rank percentile on a sorted slice.
fn percentile(sorted: &[f64], pct: f64) -> f64 {
    let rank = ((pct / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// Format `mean ± stdev` with sensible precision.
pub fn pm(s: &Summary) -> String {
    format!("{} ± {}", fmt(s.mean), fmt(s.stdev))
}

/// Format a number compactly: 3 significant digits, no trailing noise.
pub fn fmt(v: f64) -> String {
    if v == 0.0 {
        return "0".to_owned();
    }
    let magnitude = v.abs().log10().floor() as i32;
    let decimals = (2 - magnitude).clamp(0, 3) as usize;
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values are constructed, not computed
mod tests {
    use super::*;

    #[test]
    fn summary_basics() {
        let s = Summary::of(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(s.n, 5);
        assert!((s.mean - 3.0).abs() < 1e-9);
        assert_eq!(s.min, 1.0);
        assert_eq!(s.max, 5.0);
        assert_eq!(s.median, 3.0);
        assert_eq!(s.p95, 5.0);
        assert_eq!(Summary::of(&[]).n, 0);
    }

    #[test]
    fn formatting() {
        assert_eq!(fmt(0.0), "0");
        assert_eq!(fmt(1234.5), "1234");
        assert_eq!(fmt(12.345), "12.3");
        assert_eq!(fmt(0.012_345), "0.012");
        assert_eq!(fmt(3.0), "3");
    }
}
