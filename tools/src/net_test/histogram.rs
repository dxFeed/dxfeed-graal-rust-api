//! The e2e lag histogram of the NetTest summary: the bucket layout and `getValueAtPercentile` of Java's HdrHistogram
//! 2.1 as far as the tool needs them. It has the layout of the specification: the lowest value 1 ns, the highest
//! 3600 s and 3 significant digits.

/// The lowest discernible value of the histogram of the specification, in nanoseconds.
pub const HISTOGRAM_LOWEST: i64 = 1;
/// The highest value of the histogram of the specification, in nanoseconds: 3600 s.
pub const HISTOGRAM_HIGHEST: i64 = 3_600_000_000_000;
/// The significant decimal digits of the histogram of the specification.
pub const HISTOGRAM_DIGITS: u32 = 3;

/// A histogram with fixed layout. Not thread-safe: the owner guards it.
#[derive(Debug, Clone)]
pub struct Histogram {
    unit_magnitude: u32,
    sub_bucket_half_count_magnitude: u32,
    sub_bucket_count: i64,
    sub_bucket_half_count: i64,
    sub_bucket_mask: i64,
    leading_zero_count_base: u32,
    counts: Vec<i64>,
    total_count: i64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

impl Histogram {
    pub fn new() -> Self {
        let largest_value_with_single_unit_resolution = 2 * 10_i64.pow(HISTOGRAM_DIGITS);
        let unit_magnitude = 63 - (HISTOGRAM_LOWEST.max(1) as u64).leading_zeros();
        // The smallest exponent m with 2^m >= the value.
        let sub_bucket_count_magnitude = largest_value_with_single_unit_resolution
            .unsigned_abs()
            .next_power_of_two()
            .trailing_zeros();
        let sub_bucket_half_count_magnitude = sub_bucket_count_magnitude.max(1) - 1;
        let sub_bucket_count = 1_i64 << (sub_bucket_half_count_magnitude + 1);
        let sub_bucket_half_count = sub_bucket_count / 2;
        let mut smallest_untrackable_value = sub_bucket_count << unit_magnitude;
        let mut bucket_count = 1_i64;
        while smallest_untrackable_value <= HISTOGRAM_HIGHEST {
            smallest_untrackable_value <<= 1;
            bucket_count += 1;
        }
        Histogram {
            unit_magnitude,
            sub_bucket_half_count_magnitude,
            sub_bucket_count,
            sub_bucket_half_count,
            sub_bucket_mask: (sub_bucket_count - 1) << unit_magnitude,
            leading_zero_count_base: 64 - unit_magnitude - sub_bucket_half_count_magnitude - 1,
            counts: vec![0; ((bucket_count + 1) * sub_bucket_half_count) as usize],
            total_count: 0,
        }
    }

    fn bucket_index(&self, value: i64) -> u32 {
        self.leading_zero_count_base - ((value | self.sub_bucket_mask) as u64).leading_zeros()
    }

    fn sub_bucket_index(&self, value: i64, bucket: u32) -> i64 {
        value >> (bucket + self.unit_magnitude)
    }

    fn counts_index(&self, value: i64) -> usize {
        let bucket = self.bucket_index(value);
        let bucket_base_index = i64::from(bucket + 1) << self.sub_bucket_half_count_magnitude;
        (bucket_base_index + self.sub_bucket_index(value, bucket) - self.sub_bucket_half_count) as usize
    }

    fn value_from_index(&self, index: usize) -> i64 {
        let mut bucket = (index >> self.sub_bucket_half_count_magnitude) as i64 - 1;
        let mut sub_bucket = (index as i64 & (self.sub_bucket_half_count - 1)) + self.sub_bucket_half_count;
        if bucket < 0 {
            sub_bucket -= self.sub_bucket_half_count;
            bucket = 0;
        }
        sub_bucket << (bucket as u32 + self.unit_magnitude)
    }

    fn lowest_equivalent_value(&self, value: i64) -> i64 {
        let bucket = self.bucket_index(value);
        self.sub_bucket_index(value, bucket) << (bucket + self.unit_magnitude)
    }

    fn highest_equivalent_value(&self, value: i64) -> i64 {
        let bucket = self.bucket_index(value);
        let adjusted_bucket = if self.sub_bucket_index(value, bucket) >= self.sub_bucket_count {
            bucket + 1
        } else {
            bucket
        };
        self.lowest_equivalent_value(value) + (1_i64 << (self.unit_magnitude + adjusted_bucket)) - 1
    }

    /// Records a value: a negative one as 0, one above the highest as the highest.
    pub fn record(&mut self, value: i64) {
        let index = self.counts_index(value.clamp(0, HISTOGRAM_HIGHEST));
        self.counts[index] += 1;
        self.total_count += 1;
    }

    /// Adds the counts of another histogram.
    pub fn add(&mut self, other: &Histogram) {
        for (count, other) in self.counts.iter_mut().zip(&other.counts) {
            *count += other;
        }
        self.total_count += other.total_count;
    }

    /// Forgets all recorded values.
    #[cfg(test)]
    pub fn reset(&mut self) {
        self.counts.fill(0);
        self.total_count = 0;
    }

    /// The number of the recorded values.
    #[cfg(test)]
    pub fn count(&self) -> i64 {
        self.total_count
    }

    /// `getValueAtPercentile` of HdrHistogram 2.1: the highest value equivalent to the bucket where the count at the
    /// percentile (rounded to the nearest, at least 1) is reached; the lowest equivalent value for the percentile 0;
    /// 0 for an empty histogram.
    pub fn value_at_percentile(&self, percentile: f64) -> i64 {
        let requested = percentile.min(100.0);
        let count_at_percentile = ((requested / 100.0 * self.total_count as f64 + 0.5) as i64).max(1);
        let mut total = 0;
        for (index, count) in self.counts.iter().enumerate() {
            total += count;
            if total >= count_at_percentile {
                let value = self.value_from_index(index);
                return if percentile == 0.0 {
                    self.lowest_equivalent_value(value)
                } else {
                    self.highest_equivalent_value(value)
                };
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::super::kit::kit_json;
    use super::*;

    const PERCENTILES: [(&str, f64); 5] = [
        ("p50_ns", 50.0),
        ("p90_ns", 90.0),
        ("p99_ns", 99.0),
        ("p999_ns", 99.9),
        ("p9999_ns", 99.99),
    ];

    fn int(value: &Value) -> i64 {
        value
            .as_i64()
            .unwrap_or_else(|| panic!("an integer was expected: {value}"))
    }

    /// Records the `recorded_range` or the `recorded` pairs of a vector.
    fn recorded(vector: &Value) -> Histogram {
        let mut histogram = Histogram::new();
        if let Some(range) = vector.get("recorded_range") {
            for value in int(&range["from"])..=int(&range["to"]) {
                for _ in 0..int(&range["count_each"]) {
                    histogram.record(value);
                }
            }
        }
        for pair in vector.get("recorded").and_then(Value::as_array).into_iter().flatten() {
            for _ in 0..int(&pair[1]) {
                histogram.record(int(&pair[0]));
            }
        }
        histogram
    }

    #[test]
    fn the_layout_is_the_one_of_the_specification() {
        let layout = &kit_json("histogram-vectors.json")["histogram"];
        assert_eq!(int(&layout["lowest"]), HISTOGRAM_LOWEST);
        assert_eq!(int(&layout["highest"]), HISTOGRAM_HIGHEST);
        assert_eq!(int(&layout["digits"]), i64::from(HISTOGRAM_DIGITS));
    }

    #[test]
    fn the_vectors_of_the_kit_give_their_count_and_percentiles() {
        let kit = kit_json("histogram-vectors.json");
        let vectors = kit["vectors"].as_array().expect("the vectors");
        assert!(vectors.len() >= 5);
        for vector in vectors {
            let about = vector["about"].as_str().unwrap_or_default();
            let histogram = recorded(vector);
            assert_eq!(histogram.count(), int(&vector["count"]), "{about}");
            for (key, percentile) in PERCENTILES {
                assert_eq!(
                    histogram.value_at_percentile(percentile),
                    int(&vector[key]),
                    "{about}: {key}"
                );
            }
        }
    }

    #[test]
    fn an_empty_histogram_gives_zero() {
        let histogram = Histogram::new();
        assert_eq!(histogram.count(), 0);
        assert_eq!(histogram.value_at_percentile(50.0), 0);
        assert_eq!(histogram.value_at_percentile(0.0), 0);
    }

    #[test]
    fn a_negative_value_enters_as_zero_and_a_value_above_the_highest_as_the_highest() {
        let mut histogram = Histogram::new();
        histogram.record(-5);
        histogram.record(i64::MIN);
        assert_eq!(histogram.count(), 2);
        assert_eq!(histogram.value_at_percentile(100.0), 0);
        assert_eq!(histogram.value_at_percentile(0.0), 0);
        histogram.record(HISTOGRAM_HIGHEST + 1);
        histogram.record(i64::MAX);
        assert_eq!(histogram.count(), 4);
        let top = histogram.value_at_percentile(100.0);
        assert!(top >= HISTOGRAM_HIGHEST, "{top}");
        // The bucket of the highest value is 2^21 wide at most: the result stays within 0.1% of it.
        assert!(top - HISTOGRAM_HIGHEST < HISTOGRAM_HIGHEST / 1000, "{top}");
        assert_eq!(top, histogram.value_at_percentile(75.0));
        assert_eq!(histogram.value_at_percentile(50.0), 0);
    }

    #[test]
    fn a_value_keeps_three_significant_digits() {
        for value in [
            1,
            2047,
            2048,
            4095,
            381_000,
            1_000_000,
            123_456_789,
            3_000_000_000,
            HISTOGRAM_HIGHEST,
        ] {
            let mut histogram = Histogram::new();
            histogram.record(value);
            let result = histogram.value_at_percentile(50.0);
            assert!(result >= value, "{value}: {result}");
            assert!(result - value <= value / 1000, "{value}: {result}");
            // Percentile 0 is the lowest value of the bucket.
            let lowest = histogram.value_at_percentile(0.0);
            assert!(lowest <= value && value - lowest <= value / 1000, "{value}: {lowest}");
        }
    }

    #[test]
    fn add_sums_counts_and_percentiles_follow() {
        let kit = kit_json("histogram-vectors.json");
        let vectors = kit["vectors"].as_array().expect("the vectors");
        let mut total = Histogram::new();
        let mut count = 0;
        for vector in vectors {
            total.add(&recorded(vector));
            count += int(&vector["count"]);
        }
        assert_eq!(total.count(), count);
        // The same values recorded one by one in another order give the same percentiles.
        let mut direct = Histogram::new();
        for vector in vectors.iter().rev() {
            direct.add(&recorded(vector));
        }
        for (_, percentile) in PERCENTILES {
            assert_eq!(
                total.value_at_percentile(percentile),
                direct.value_at_percentile(percentile)
            );
        }
        // Adding changes neither the added histogram nor itself otherwise.
        let other = recorded(&vectors[1]);
        let mut sum = recorded(&vectors[1]);
        sum.add(&other);
        assert_eq!(sum.count(), 2);
        assert_eq!(other.count(), 1);
        assert_eq!(sum.value_at_percentile(50.0), other.value_at_percentile(50.0));
    }

    #[test]
    fn reset_forgets_everything_and_the_histogram_is_reusable() {
        let kit = kit_json("histogram-vectors.json");
        let vector = &kit["vectors"][2];
        let mut histogram = recorded(vector);
        assert!(histogram.count() > 0);
        histogram.reset();
        assert_eq!(histogram.count(), 0);
        assert_eq!(histogram.value_at_percentile(99.0), 0);
        histogram.record(381_000);
        assert_eq!(histogram.value_at_percentile(50.0), 381_183);
    }

    #[test]
    fn a_percentile_above_100_is_the_maximum() {
        let mut histogram = Histogram::new();
        for value in [10, 20, 30] {
            histogram.record(value);
        }
        assert_eq!(
            histogram.value_at_percentile(250.0),
            histogram.value_at_percentile(100.0)
        );
        assert_eq!(histogram.value_at_percentile(100.0), 30);
        assert_eq!(histogram.value_at_percentile(0.0), 10);
    }
}
