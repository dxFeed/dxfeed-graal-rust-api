//! The symbol pool of NetTest: the symbols of an IPF file or generated ones.

use std::collections::HashSet;
use std::sync::Arc;

use dxfeed_graal::ipf::InstrumentProfileReader;

use super::cli::{MAX_SYMBOLS, Options};

/// The seed of the symbol generator of QD NetTest.
const SYMBOL_SEED: i64 = 1_416_948_710_541_751;

const RANDOM_MULTIPLIER: u64 = 0x5_DEEC_E66D;
const RANDOM_ADDEND: u64 = 0xB;
const RANDOM_MASK: u64 = (1 << 48) - 1;

/// `java.util.Random` as its Javadoc specifies it: the linear congruential generator of 48 bits.
struct JavaRandom {
    seed: u64,
}

impl JavaRandom {
    fn new(seed: i64) -> Self {
        JavaRandom {
            seed: (seed as u64 ^ RANDOM_MULTIPLIER) & RANDOM_MASK,
        }
    }

    /// The top `bits` of the next 48-bit state, as a signed 32-bit integer.
    fn next(&mut self, bits: u32) -> i32 {
        self.seed = self.seed.wrapping_mul(RANDOM_MULTIPLIER).wrapping_add(RANDOM_ADDEND) & RANDOM_MASK;
        (self.seed >> (48 - bits)) as i32
    }

    /// A value in `0..bound` for a positive bound; draws again while the draw falls into the incomplete last block of
    /// the 31-bit range, as Java does with its overflow check.
    fn next_int(&mut self, bound: i32) -> i32 {
        let mut draw = self.next(31);
        let mask = bound - 1;
        if bound & mask == 0 {
            return ((i64::from(bound) * i64::from(draw)) >> 31) as i32;
        }
        loop {
            let value = draw % bound;
            if draw.wrapping_sub(value).wrapping_add(mask) >= 0 {
                return value;
            }
            draw = self.next(31);
        }
    }
}

/// The symbol generator of QD NetTest (`SymbolGenerator`): 1 to 6 characters, capital letters with an occasional digit.
struct SymbolGenerator {
    random: JavaRandom,
}

impl SymbolGenerator {
    fn new() -> Self {
        SymbolGenerator {
            random: JavaRandom::new(SYMBOL_SEED),
        }
    }

    /// The next symbol of the sequence; the sequence repeats symbols from time to time.
    fn next_symbol(&mut self) -> String {
        // The two draws of the length are made in this order.
        let short = self.random.next_int(3);
        let length = short + self.random.next_int(4) + 1;
        (0..length)
            .map(|_| {
                if self.random.next_int(50) == 0 {
                    char::from(b'0' + self.random.next_int(10) as u8)
                } else {
                    char::from(b'A' + self.random.next_int(26) as u8)
                }
            })
            .collect()
    }
}

/// The symbols of a pool, shared by the event pools of all instances.
pub type Symbols = Arc<Vec<String>>;

/// A built pool: the symbols and a warning to print.
#[derive(Debug)]
pub struct SymbolPool {
    pub symbols: Symbols,
    pub warning: Option<String>,
}

/// `total` distinct symbols of QD NetTest in the order of their first appearance, the same on every run and platform;
/// empty when `total` is below 1 or above the limit of the tool.
pub fn generate_symbols(total: i64) -> Vec<String> {
    if !(1..=MAX_SYMBOLS).contains(&total) {
        return Vec::new();
    }
    let total = total as usize;
    let mut symbols = Vec::with_capacity(total.min(1 << 20));
    let mut seen = HashSet::with_capacity(total.min(1 << 20));
    let mut generator = SymbolGenerator::new();
    while symbols.len() < total {
        let symbol = generator.next_symbol();
        if !seen.contains(&symbol) {
            seen.insert(symbol.clone());
            symbols.push(symbol);
        }
    }
    symbols
}

/// The symbols of the consumer instance `instance` (from 1) of `count`: those with the index `j` (from 0) where
/// `j % count == instance - 1`, so that the instances together take each symbol once. An instance outside `1..=count`
/// has none.
pub fn consumer_slice(symbols: &[String], instance: usize, count: usize) -> Vec<String> {
    if instance < 1 || instance > count {
        return Vec::new();
    }
    symbols.iter().skip(instance - 1).step_by(count).cloned().collect()
}

/// The first `total` symbols of the profiles of the IPF file, in file order. Fewer give the smaller pool and a
/// warning; no symbols, a missing or an unreadable file is an error.
pub fn read_symbols_from_file(file: &str, total: i64) -> Result<SymbolPool, String> {
    let reader = InstrumentProfileReader::new().map_err(|error| format!("cannot create the IPF reader: {error}"))?;
    let profiles = reader
        .read_from_file(file)
        .map_err(|error| format!("cannot read the IPF file '{file}': {error}"))?;
    let symbols: Vec<String> = profiles
        .into_iter()
        .map(|profile| profile.symbol)
        .filter(|symbol| !symbol.is_empty())
        .take(usize::try_from(total).unwrap_or(usize::MAX))
        .collect();
    if symbols.is_empty() {
        return Err(format!("the IPF file '{file}' has no symbols"));
    }
    let warning = ((symbols.len() as i64) < total).then(|| {
        format!(
            "the IPF file '{file}' has {} symbols, fewer than the requested {total}",
            symbols.len()
        )
    });
    Ok(SymbolPool {
        symbols: Arc::new(symbols),
        warning,
    })
}

/// The pool of the options: from the IPF file when the options name one, generated otherwise.
pub fn build_symbol_pool(options: &Options) -> Result<SymbolPool, String> {
    if let Some(file) = &options.symbols_file {
        return read_symbols_from_file(file, options.total_symbols);
    }
    let symbols = generate_symbols(options.total_symbols);
    if symbols.is_empty() {
        return Err(format!("cannot generate {} distinct symbols", options.total_symbols));
    }
    Ok(SymbolPool {
        symbols: Arc::new(symbols),
        warning: None,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::net_test::kit::{kit_hex, kit_json, kit_text};

    /// A file in the temporary directory, removed when the test ends.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str, content: &str) -> Self {
            let path = std::env::temp_dir().join(format!("dxfeed-graal-tools-{}-{name}", std::process::id()));
            std::fs::write(&path, content).unwrap();
            TempFile(path)
        }

        fn path(&self) -> String {
            self.0.display().to_string()
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    const FIXTURE: &str = "##0 five symbols, deliberately not in alphabetical order\n\
                           #STOCK::=TYPE,SYMBOL,DESCRIPTION\n\
                           STOCK,IBM,International Business Machines\n\
                           STOCK,AAPL,Apple Inc\n\
                           STOCK,MSFT,Microsoft Corp\n\
                           STOCK,GOOG,Alphabet Inc\n\
                           STOCK,TSLA,Tesla Inc\n\
                           ##COMPLETE\n";

    /// The 64-bit FNV-1a hash of the symbols, each followed by a line feed, as the kit defines it.
    fn fnv1a(symbols: &[String]) -> u64 {
        let mut hash = 0xCBF2_9CE4_8422_2325_u64;
        for symbol in symbols {
            for byte in symbol.bytes().chain(std::iter::once(b'\n')) {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01B3);
            }
        }
        hash
    }

    #[test]
    fn the_first_ten_generated_symbols_are_those_of_the_kit() {
        let expected: Vec<String> = kit_text("symbols-generated-10.txt")
            .lines()
            .map(str::to_owned)
            .collect();
        assert_eq!(expected.len(), 10);
        assert_eq!(generate_symbols(10), expected);
    }

    #[test]
    fn the_generated_symbols_equal_the_kit_vectors() {
        let vectors = kit_json("symbols-generated-vectors.json");
        let cases = vectors["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 3);
        for case in cases {
            let total = case["total"].as_i64().unwrap();
            let symbols = generate_symbols(total);
            assert_eq!(symbols.len() as i64, total);
            assert_eq!(fnv1a(&symbols), kit_hex(&case["fnv1a64"]), "the hash of {total}");
            for sample in case["samples"].as_array().unwrap() {
                let index = sample["index"].as_u64().unwrap() as usize;
                assert_eq!(symbols[index], sample["symbol"].as_str().unwrap(), "{total}: {index}");
            }
            // The draws of the kit: the symbols drawn, duplicates included, until `total` distinct ones appear.
            let mut generator = SymbolGenerator::new();
            let mut seen = HashSet::new();
            let mut draws = 0_i64;
            while (seen.len() as i64) < total {
                seen.insert(generator.next_symbol());
                draws += 1;
            }
            assert_eq!(draws, case["draws"].as_i64().unwrap(), "the draws of {total}");
        }
    }

    #[test]
    fn a_smaller_set_is_the_prefix_of_a_bigger_one() {
        let big = generate_symbols(100_000);
        for total in [1, 10, 999, 1000, 54_321] {
            assert_eq!(generate_symbols(total), big[..total as usize], "{total}");
        }
    }

    #[test]
    fn the_generated_symbols_are_distinct_and_of_one_to_six_characters() {
        let symbols = generate_symbols(5000);
        assert_eq!(symbols.iter().collect::<HashSet<_>>().len(), 5000);
        let mut lengths = HashSet::new();
        let mut digits = 0;
        for symbol in &symbols {
            assert!((1..=6).contains(&symbol.len()), "{symbol}");
            assert!(
                symbol
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()),
                "{symbol}"
            );
            digits += symbol.bytes().filter(u8::is_ascii_digit).count();
            lengths.insert(symbol.len());
        }
        assert_eq!(lengths.len(), 6);
        // About one character in 50 is a digit.
        assert!(digits > 0, "no digits in the symbols");
    }

    #[test]
    fn the_java_random_matches_the_javadoc() {
        // `new java.util.Random(42).nextInt()` is -1170105035 on the Java platform.
        assert_eq!(JavaRandom::new(42).next(32), -1_170_105_035);
        // The first `next(31)` is 1562431130: a bound that is not a power of two takes the remainder, a power of two
        // the high bits.
        assert_eq!(JavaRandom::new(42).next_int(10), 0);
        assert_eq!(JavaRandom::new(42).next_int(7), 1_562_431_130 % 7);
        assert_eq!(JavaRandom::new(42).next_int(16), 11);
    }

    #[test]
    fn a_bound_near_the_int_limit_rejects_the_draws_that_overflow() {
        // With a bound above 2^30 the check `draw - draw % bound + bound - 1` overflows a signed 32-bit integer for
        // the draws of the last partial block; those are drawn again, so the result always stays below the bound.
        let mut random = JavaRandom::new(7);
        for _ in 0..1000 {
            let value = random.next_int((1 << 30) + 1);
            assert!((0..=(1 << 30)).contains(&value), "{value}");
        }
    }

    #[test]
    fn the_consumer_slices_cover_the_set_once() {
        let symbols: Vec<String> = ["A", "B", "C", "D", "E"].map(str::to_owned).to_vec();
        assert_eq!(consumer_slice(&symbols, 1, 2), ["A", "C", "E"]);
        assert_eq!(consumer_slice(&symbols, 2, 2), ["B", "D"]);
        assert_eq!(consumer_slice(&symbols, 1, 1), symbols);
        // More instances than symbols leave the last ones empty.
        assert!(consumer_slice(&symbols, 6, 7).is_empty());
        assert_eq!(consumer_slice(&symbols, 5, 7), ["E"]);
        // An instance outside 1..=count has nothing.
        assert!(consumer_slice(&symbols, 0, 2).is_empty());
        assert!(consumer_slice(&symbols, 3, 2).is_empty());
        assert!(consumer_slice(&symbols, 1, 0).is_empty());
        let big = generate_symbols(1000);
        let mut union: Vec<String> = (1..=7).flat_map(|instance| consumer_slice(&big, instance, 7)).collect();
        union.sort();
        let mut expected = big.clone();
        expected.sort();
        assert_eq!(union, expected);
    }

    #[test]
    fn an_impossible_total_fails() {
        assert!(generate_symbols(0).is_empty());
        assert!(generate_symbols(-1).is_empty());
        assert!(generate_symbols(1_000_000_000).is_empty());
        let options = Options {
            total_symbols: 1_000_000_000,
            ..Options::default()
        };
        assert_eq!(
            build_symbol_pool(&options).unwrap_err(),
            "cannot generate 1000000000 distinct symbols"
        );
    }

    #[test]
    fn the_ipf_symbols_in_file_order() {
        let file = TempFile::new("symbols-order.ipf", FIXTURE);
        let pool = read_symbols_from_file(&file.path(), 3).unwrap();
        assert_eq!(*pool.symbols, ["IBM", "AAPL", "MSFT"]);
        assert_eq!(pool.warning, None);

        let options = Options {
            total_symbols: 2,
            symbols_file: Some(file.path()),
            ..Options::default()
        };
        assert_eq!(*build_symbol_pool(&options).unwrap().symbols, ["IBM", "AAPL"]);
    }

    #[test]
    fn fewer_ipf_symbols_than_the_total_give_a_warning() {
        let file = TempFile::new("symbols-fewer.ipf", FIXTURE);
        let pool = read_symbols_from_file(&file.path(), 10).unwrap();
        assert_eq!(*pool.symbols, ["IBM", "AAPL", "MSFT", "GOOG", "TSLA"]);
        assert_eq!(
            pool.warning.unwrap(),
            format!(
                "the IPF file '{}' has 5 symbols, fewer than the requested 10",
                file.path()
            )
        );
    }

    #[test]
    fn an_empty_or_missing_ipf_file_fails() {
        let empty = TempFile::new("symbols-empty.ipf", "#STOCK::=TYPE,SYMBOL\n##COMPLETE\n");
        assert_eq!(
            read_symbols_from_file(&empty.path(), 10).unwrap_err(),
            format!("the IPF file '{}' has no symbols", empty.path())
        );
        let missing = format!("{}.missing", empty.path());
        let error = read_symbols_from_file(&missing, 10).unwrap_err();
        assert!(
            error.starts_with(&format!("cannot read the IPF file '{missing}': ")),
            "{error}"
        );
    }
}
