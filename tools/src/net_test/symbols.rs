//! The symbol pool of NetTest: the symbols of an IPF file or generated ones.

use std::collections::HashSet;
use std::sync::Arc;

use dxfeed_graal::ipf::InstrumentProfileReader;

use super::cli::Options;

const SYMBOL_SEED: u64 = 0x5EED_0FD4_F3ED_5EED;
const MAX_GENERATED: i64 =
    26 + 26 * 26 + 26 * 26 * 26 + 26 * 26 * 26 * 26 + 26 * 26 * 26 * 26 * 26 + 26 * 26 * 26 * 26 * 26 * 26;

/// SplitMix64: a small generator that gives the same numbers on every platform.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
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

/// `total` distinct symbols of 1-6 capital letters from a fixed seed, the same on every run and platform; empty when
/// `total` is below 1 or above the number of the possible symbols.
pub fn generate_symbols(total: i64) -> Vec<String> {
    if !(1..=MAX_GENERATED).contains(&total) {
        return Vec::new();
    }
    let total = total as usize;
    let mut symbols = Vec::with_capacity(total);
    let mut seen = HashSet::with_capacity(total);
    let mut random = SplitMix64(SYMBOL_SEED);
    while symbols.len() < total {
        let length = (random.next() % 6) as usize + 1;
        let symbol: String = (0..length)
            .map(|_| char::from(b'A' + (random.next() % 26) as u8))
            .collect();
        if seen.insert(symbol.clone()) {
            symbols.push(symbol);
        }
    }
    symbols
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

    #[test]
    fn generated_symbols_are_distinct_capital_letters_and_deterministic() {
        let first = generate_symbols(1000);
        assert_eq!(first.len(), 1000);
        assert_eq!(first, generate_symbols(1000));
        assert_eq!(first.iter().collect::<HashSet<_>>().len(), 1000);
        let mut lengths = HashSet::new();
        for symbol in &first {
            assert!((1..=6).contains(&symbol.len()), "{symbol}");
            assert!(symbol.bytes().all(|letter| letter.is_ascii_uppercase()), "{symbol}");
            lengths.insert(symbol.len());
        }
        assert_eq!(lengths.len(), 6);
        // A smaller pool is the start of a bigger one.
        assert_eq!(generate_symbols(10), first[..10]);
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
