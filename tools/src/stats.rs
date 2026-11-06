//! The statistics of the delays that LatencyTest measures, computed as the C++ tool computes them.

/// The statistics of a sample of delays, in milliseconds; every value is NaN for an empty sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub min: f64,
    pub max: f64,
    /// The 99th percentile, interpolated as Excel's `PERCENTILE` does.
    pub percentile: f64,
    pub mean: f64,
    /// The sample standard deviation; zero for one value.
    pub std_dev: f64,
    /// The standard error of the mean.
    pub std_err: f64,
    pub size: usize,
}

impl Stats {
    /// The statistics of `values`, which it sorts.
    pub fn of(values: &mut [i64]) -> Stats {
        if values.is_empty() {
            return Stats {
                min: f64::NAN,
                max: f64::NAN,
                percentile: f64::NAN,
                mean: f64::NAN,
                std_dev: f64::NAN,
                std_err: f64::NAN,
                size: 0,
            };
        }
        values.sort_unstable();
        let count = values.len() as f64;
        let mean = values.iter().map(|&v| v as f64).sum::<f64>() / count;
        let std_dev = if values.len() <= 1 {
            0.0
        } else {
            let squares: f64 = values.iter().map(|&v| (v as f64 - mean) * (v as f64 - mean)).sum();
            (squares / (count - 1.0)).sqrt()
        };
        Stats {
            min: values[0] as f64,
            max: values[values.len() - 1] as f64,
            percentile: percentile(values, 0.99),
            mean,
            std_dev,
            std_err: std_dev / count.sqrt(),
            size: values.len(),
        }
    }
}

/// The `share` percentile (from 0 to 1) of non-empty `sorted` values, interpolated as Excel's `PERCENTILE` does.
fn percentile(sorted: &[i64], share: f64) -> f64 {
    let rank = (sorted.len() as f64 - 1.0) * share + 1.0;
    if sorted.len() == 1 || (rank - 1.0).abs() < f64::EPSILON {
        return sorted[0] as f64;
    }
    let k = rank as usize;
    if k >= sorted.len() {
        return sorted[sorted.len() - 1] as f64;
    }
    let fraction = rank - k as f64;
    sorted[k - 1] as f64 + fraction * (sorted[k] as f64 - sorted[k - 1] as f64)
}

/// A value with two decimals, NaN as `---`.
pub fn format(value: f64) -> String {
    if value.is_nan() {
        "---".to_owned()
    } else {
        format!("{value:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn statistics_of_four_values() {
        let stats = Stats::of(&mut [4, 1, 3, 2]);
        assert_eq!((stats.min, stats.max, stats.mean, stats.size), (1.0, 4.0, 2.5, 4));
        // Excel: PERCENTILE({1,2,3,4}, 0.99) = 3 + 0.97 * (4 - 3).
        assert!(close(stats.percentile, 3.97), "{stats:?}");
        // The sample variance is (2.25 + 0.25 + 0.25 + 2.25) / 3 = 5 / 3.
        assert!(close(stats.std_dev, (5.0f64 / 3.0).sqrt()), "{stats:?}");
        assert!(close(stats.std_err, stats.std_dev / 2.0), "{stats:?}");
    }

    #[test]
    fn one_value_has_no_spread() {
        let stats = Stats::of(&mut [7]);
        assert_eq!(
            (stats.min, stats.max, stats.percentile, stats.mean),
            (7.0, 7.0, 7.0, 7.0)
        );
        assert_eq!((stats.std_dev, stats.std_err), (0.0, 0.0));
    }

    #[test]
    fn no_values_have_no_statistics() {
        let stats = Stats::of(&mut []);
        assert_eq!(stats.size, 0);
        assert_eq!(format(stats.min), "---");
        assert_eq!(format(stats.std_err), "---");
        assert_eq!(format(-1.234), "-1.23");
    }

    #[test]
    fn extreme_values_do_not_overflow() {
        let stats = Stats::of(&mut [i64::MAX, i64::MIN]);
        assert_eq!((stats.min, stats.max), (i64::MIN as f64, i64::MAX as f64));
        assert!(stats.percentile.is_finite() && stats.percentile > 0.0, "{stats:?}");
    }

    #[test]
    fn the_percentile_of_many_values() {
        let values: Vec<i64> = (1..=100).collect();
        // Excel: PERCENTILE(1..100, 0.99) = 99 + 0.01 * (100 - 99).
        assert!(close(percentile(&values, 0.99), 99.01));
        assert!(close(percentile(&values, 0.0), 1.0));
        assert!(close(percentile(&values, 1.0), 100.0));
    }
}
