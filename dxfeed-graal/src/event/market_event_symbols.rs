//! Helpers for the symbols of market events, as Java's `MarketEventSymbols` has them.
//!
//! A market event symbol is a base symbol, an optional exchange code after `&` and optional attributes in braces:
//! `IBM&N{price=bid}`. The attributes are `key=value` pairs separated by commas and sorted by key.

use crate::error::{Error, Result};
use crate::event::format::JavaDouble;

/// The Java regular expression of the exchange codes the API supports (Java's `EXCHANGES_PATTERN`).
pub const EXCHANGES_PATTERN: &str = "[0-9A-Za-z]";

/// All the exchange codes the API supports: digits and Latin letters (Java's `SUPPORTED_EXCHANGES`).
pub const SUPPORTED_EXCHANGES: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// The exchange codes used by default: the capital Latin letters (Java's `DEFAULT_EXCHANGES`).
pub const DEFAULT_EXCHANGES: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";

const EXCHANGE_SEPARATOR: char = '&';
const ATTRIBUTES_OPEN: char = '{';
const ATTRIBUTES_CLOSE: char = '}';
const ATTRIBUTES_SEPARATOR: char = ',';
const ATTRIBUTE_VALUE: char = '=';

/// Whether `symbol` has an exchange code: `&` and one character before the attributes.
#[doc(alias = "hasExchangeCode")]
pub fn has_exchange_code(symbol: &str) -> bool {
    exchange_code_of(symbol, length_without_attributes(symbol)).is_some()
}

/// The exchange code of `symbol`, `'\0'` if it has none.
#[doc(alias = "getExchangeCode")]
pub fn get_exchange_code(symbol: &str) -> char {
    exchange_code_of(symbol, length_without_attributes(symbol)).map_or('\0', |(_, code)| code)
}

/// `symbol` with the exchange code `exchange_code`, or without an exchange code if it is `'\0'`; the attributes stay.
#[doc(alias = "changeExchangeCode")]
pub fn change_exchange_code(symbol: &str, exchange_code: char) -> String {
    let length = length_without_attributes(symbol);
    let mut result = base_symbol(symbol, length).to_owned();
    if exchange_code != '\0' {
        result.push(EXCHANGE_SEPARATOR);
        result.push(exchange_code);
    }
    result + &symbol[length..]
}

/// The base symbol of `symbol`, without the exchange code and the attributes.
#[doc(alias = "getBaseSymbol")]
pub fn get_base_symbol(symbol: &str) -> &str {
    base_symbol(symbol, length_without_attributes(symbol))
}

/// `symbol` with the base symbol `base_symbol`; the exchange code and the attributes stay.
#[doc(alias = "changeBaseSymbol")]
pub fn change_base_symbol(symbol: &str, base_symbol: &str) -> String {
    let length = length_without_attributes(symbol);
    let mut result = base_symbol.to_owned();
    if let Some((_, code)) = exchange_code_of(symbol, length) {
        result.push(EXCHANGE_SEPARATOR);
        result.push(code);
    }
    result + &symbol[length..]
}

/// Whether `symbol` has attributes in braces.
#[doc(alias = "hasAttributes")]
pub fn has_attributes(symbol: &str) -> bool {
    length_without_attributes(symbol) < symbol.len()
}

/// The value of the attribute `key` of `symbol`, `None` if it has none.
#[doc(alias = "getAttributeStringByKey")]
pub fn get_attribute_string_by_key<'a>(symbol: &'a str, key: &str) -> Option<&'a str> {
    let length = length_without_attributes(symbol);
    if length == symbol.len() {
        return None;
    }
    let mut i = length + 1;
    while i < symbol.len() {
        let current = key_at(symbol, i)?;
        let j = next_key(symbol, i);
        if current == key {
            return Some(value_at(symbol, i, j));
        }
        i = j;
    }
    None
}

/// `symbol` with the attribute `key` set to `value`, or without it if `value` is `None`; the attributes stay sorted
/// by key.
#[doc(alias = "changeAttributeStringByKey")]
pub fn change_attribute_string_by_key(symbol: &str, key: &str, value: Option<&str>) -> String {
    let length = length_without_attributes(symbol);
    match value {
        None => remove_attribute(symbol, length, key),
        Some(value) if length == symbol.len() => format!("{symbol}{{{key}={value}}}"),
        Some(value) => add_attribute(symbol, length, key, value),
    }
}

/// `symbol` without the attribute `key`; the braces go when the last attribute goes.
#[doc(alias = "removeAttributeStringByKey")]
pub fn remove_attribute_string_by_key(symbol: &str, key: &str) -> String {
    remove_attribute(symbol, length_without_attributes(symbol), key)
}

/// The symbol of a spread of the legs in `spread`, each a symbol with its ratio, as Java's `buildSpreadSymbol`
/// builds it: `=` and the legs with a non-zero ratio sorted by ratio, then by symbol, such as `=2*AAPL-IBM`.
///
/// No legs give an empty string, one leg with ratio 1 gives its symbol. Unlike the keys of a Java map, a symbol may
/// come twice: each leg is written.
#[doc(alias = "buildSpreadSymbol")]
pub fn build_spread_symbol<I, S, R>(spread: I) -> String
where
    I: IntoIterator<Item = (S, R)>,
    S: AsRef<str>,
    R: std::borrow::Borrow<f64>,
{
    let mut legs: Vec<(S, f64)> = Vec::new();
    for (symbol, ratio) in spread {
        let ratio = *ratio.borrow();
        if ratio != 0.0 {
            // An insertion sort keeps Java's order for equal legs; with a NaN ratio, which Java's sort orders by
            // the order of the map, the order is unspecified.
            let at = legs
                .iter()
                .position(|leg| compare_legs((&symbol, ratio), (&leg.0, leg.1)).is_lt())
                .unwrap_or(legs.len());
            legs.insert(at, (symbol, ratio));
        }
    }
    match legs.as_slice() {
        [] => return String::new(),
        [(symbol, ratio)] if *ratio == 1.0 => return symbol.as_ref().to_owned(),
        _ => {}
    }
    let mut result = String::from("=");
    for (symbol, ratio) in &legs {
        if *ratio < 0.0 {
            result.push('-');
        } else if result.len() > 1 {
            result.push('+');
        }
        let ratio = ratio.abs();
        if ratio != 1.0 {
            if ratio == f64::from(ratio as i32) {
                result.push_str(&(ratio as i32).to_string());
            } else {
                result.push_str(&JavaDouble(ratio).to_string());
            }
            result.push('*');
        }
        result.push_str(symbol.as_ref());
    }
    result
}

/// The exchange codes in 0 to 127 that match `character_class`, a Java character class such as `A-Z`, `[A-DQ]` or
/// `\d`; a class without brackets gets them.
///
/// This port reads one class: literal characters, ranges, negation, nested classes, intersections with `&&`, the
/// classes `\d`, `\D`, `\w`, `\W`, `\s` and `\S`, the escapes `\t`, `\n`, `\r` and `\f`, and a backslash before a
/// character other than a letter or a digit. Other escapes (`\x41`, `\u0041`, `\p{Upper}` and so on) and anything
/// after the class, such as the `+` of `\d+`, are refused.
///
/// # Errors
///
/// [`Error::InvalidArgument`] if `character_class` is not such a class or matches a code that is not a digit or a
/// Latin letter.
#[doc(alias = "getExchangesByPattern")]
pub fn get_exchanges_by_pattern(character_class: &str) -> Result<String> {
    if character_class.is_empty() {
        return Ok(String::new());
    }
    let class = if character_class.starts_with(['[', '\\']) {
        character_class.to_owned()
    } else {
        format!("[{character_class}]")
    };
    let set = parse_class(&class)?;
    let exchanges: String = (0..=127u8).filter(|&c| set[usize::from(c)]).map(char::from).collect();
    let unsupported: Vec<String> = exchanges
        .chars()
        .filter(|c| !c.is_ascii_alphanumeric())
        .map(String::from)
        .collect();
    if unsupported.is_empty() {
        Ok(exchanges)
    } else {
        Err(Error::InvalidArgument(format!(
            "Unsupported exchange codes: {}",
            unsupported.join(", ")
        )))
    }
}

/// Java's `SpreadLeg.compareTo`: the larger ratio first, then the symbols in order for a positive ratio and in
/// reverse order for a negative one.
fn compare_legs<S: AsRef<str>>(a: (&S, f64), b: (&S, f64)) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if a.1 > b.1 {
        Ordering::Less
    } else if a.1 < b.1 {
        Ordering::Greater
    } else {
        let order = a.0.as_ref().cmp(b.0.as_ref());
        if a.1 >= 0.0 { order } else { order.reverse() }
    }
}

/// The byte position of the exchange separator and the exchange code in `symbol[..length]`, if it has them.
fn exchange_code_of(symbol: &str, length: usize) -> Option<(usize, char)> {
    let mut chars = symbol[..length].char_indices().rev();
    let (_, code) = chars.next()?;
    let (separator, c) = chars.next()?;
    // Java reads UTF-16: a code beyond U+FFFF is two units, and the unit before the last one is not `&`.
    (c == EXCHANGE_SEPARATOR && u32::from(code) <= 0xFFFF).then_some((separator, code))
}

fn base_symbol(symbol: &str, length: usize) -> &str {
    match exchange_code_of(symbol, length) {
        Some((separator, _)) => &symbol[..separator],
        None => &symbol[..length],
    }
}

/// The length of `symbol` without the attributes: the position of the last `{` if the symbol ends with a
/// non-empty `{...}`.
fn length_without_attributes(symbol: &str) -> usize {
    let length = symbol.len();
    if length >= 3
        && symbol.ends_with(ATTRIBUTES_CLOSE)
        && let Some(open) = symbol[..length - 1].rfind(ATTRIBUTES_OPEN)
        && open < length - 2
    {
        return open;
    }
    length
}

/// The key of the attribute at `i`, `None` if there is no `=` after it.
fn key_at(symbol: &str, i: usize) -> Option<&str> {
    symbol[i..].find(ATTRIBUTE_VALUE).map(|value| &symbol[i..i + value])
}

/// The position after the attribute at `i`: after its comma, or the end of the symbol.
fn next_key(symbol: &str, i: usize) -> usize {
    let value = symbol[i..].find(ATTRIBUTE_VALUE).map_or(0, |v| i + v + 1);
    symbol[value..]
        .find(ATTRIBUTES_SEPARATOR)
        .map_or(symbol.len(), |separator| value + separator + 1)
}

/// The value of the attribute at `i` that ends before `j`.
fn value_at(symbol: &str, i: usize, j: usize) -> &str {
    let value = i + symbol[i..].find(ATTRIBUTE_VALUE).unwrap_or(0) + 1;
    &symbol[value..j - 1]
}

/// `symbol` without the attribute at `i` that ends before `j`.
fn drop_attribute(symbol: &str, length: usize, i: usize, j: usize) -> String {
    if j == symbol.len() {
        if i == length + 1 {
            symbol[..length].to_owned()
        } else {
            format!("{}{}", &symbol[..i - 1], &symbol[j - 1..])
        }
    } else {
        format!("{}{}", &symbol[..i], &symbol[j..])
    }
}

fn remove_attribute(symbol: &str, length: usize, key: &str) -> String {
    let mut symbol = symbol.to_owned();
    if length == symbol.len() {
        return symbol;
    }
    let mut i = length + 1;
    while i < symbol.len() {
        let Some(current) = key_at(&symbol, i) else {
            break;
        };
        let j = next_key(&symbol, i);
        if current == key {
            symbol = drop_attribute(&symbol, length, i, j);
        } else {
            i = j;
        }
    }
    symbol
}

fn add_attribute(symbol: &str, length: usize, key: &str, value: &str) -> String {
    let mut symbol = symbol.to_owned();
    let mut i = length + 1;
    let mut added = false;
    while i < symbol.len() {
        let Some(current) = key_at(&symbol, i) else {
            break;
        };
        let j = next_key(&symbol, i);
        let order = current.cmp(key);
        if order.is_eq() {
            if added {
                symbol = drop_attribute(&symbol, length, i, j);
            } else {
                symbol = format!("{}{key}={value}{}", &symbol[..i], &symbol[j - 1..]);
                added = true;
                i += key.len() + value.len() + 2;
            }
        } else if order.is_gt() && !added {
            symbol = format!("{}{key}={value},{}", &symbol[..i], &symbol[i..]);
            added = true;
            i += key.len() + value.len() + 2;
        } else {
            i = j;
        }
    }
    if added {
        symbol
    } else {
        format!("{},{key}={value}{}", &symbol[..i - 1], &symbol[i - 1..])
    }
}

/// The ASCII characters a Java character class matches.
type CharSet = [bool; 128];

/// Parses a Java character class: `\d` and the like, or brackets with characters, ranges, escapes, nested classes,
/// intersections with `&&` and negation with `^`.
fn parse_class(class: &str) -> Result<CharSet> {
    let unsupported = || Error::InvalidArgument(format!("unsupported character class {class:?}"));
    let mut chars = class.chars().peekable();
    let set = match chars.next() {
        Some('\\') => match escape(&mut chars).ok_or_else(unsupported)? {
            Escape::Set(set) => set,
            Escape::Char(c) => range(c, c),
        },
        Some('[') => bracket(&mut chars).ok_or_else(unsupported)?,
        _ => return Err(unsupported()),
    };
    match chars.next() {
        None => Ok(set),
        Some(_) => Err(unsupported()),
    }
}

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

/// The rest of a bracketed class after its `[`, up to and with its `]`.
fn bracket(chars: &mut Chars<'_>) -> Option<CharSet> {
    let negated = chars.next_if_eq(&'^').is_some();
    let mut intersection: Option<CharSet> = None;
    let mut union = [false; 128];
    let mut empty = true;
    // Whether the operand of `&&` being read has an item; as in Java, an empty operand is left out.
    let mut operand = false;
    loop {
        let first = match chars.next()? {
            ']' if !empty => break,
            '[' => {
                union = or(union, bracket(chars)?);
                (empty, operand) = (false, true);
                continue;
            }
            '&' if chars.peek() == Some(&'&') => {
                chars.next();
                if operand {
                    intersection = Some(intersection.map_or(union, |set| and(set, union)));
                }
                (union, operand) = ([false; 128], false);
                continue;
            }
            '\\' => match escape(chars)? {
                Escape::Set(set) => {
                    union = or(union, set);
                    (empty, operand) = (false, true);
                    continue;
                }
                Escape::Char(c) => c,
            },
            c => c,
        };
        let mut last = first;
        let mut ahead = chars.clone();
        if ahead.next() == Some('-') && ahead.peek().is_some_and(|&c| c != ']' && c != '[') {
            chars.next();
            last = match chars.next()? {
                '\\' => match escape(chars)? {
                    Escape::Char(c) => c,
                    Escape::Set(_) => return None,
                },
                c => c,
            };
            if last < first {
                return None;
            }
        }
        union = or(union, range(first, last));
        (empty, operand) = (false, true);
    }
    let set = match intersection {
        Some(set) if operand => and(set, union),
        Some(set) => set,
        None => union,
    };
    Some(if negated { set.map(|c| !c) } else { set })
}

enum Escape {
    Set(CharSet),
    Char(char),
}

/// The class or the character after a backslash; a letter or a digit without a meaning is not supported.
fn escape(chars: &mut Chars<'_>) -> Option<Escape> {
    let digit = range('0', '9');
    let word = or(or(digit, range('A', 'Z')), or(range('a', 'z'), range('_', '_')));
    let space = [' ', '\t', '\n', '\x0B', '\x0C', '\r']
        .into_iter()
        .fold([false; 128], |set, c| or(set, range(c, c)));
    Some(match chars.next()? {
        'd' => Escape::Set(digit),
        'D' => Escape::Set(digit.map(|c| !c)),
        'w' => Escape::Set(word),
        'W' => Escape::Set(word.map(|c| !c)),
        's' => Escape::Set(space),
        'S' => Escape::Set(space.map(|c| !c)),
        't' => Escape::Char('\t'),
        'n' => Escape::Char('\n'),
        'r' => Escape::Char('\r'),
        'f' => Escape::Char('\x0C'),
        c if c.is_ascii_alphanumeric() => return None,
        c => Escape::Char(c),
    })
}

/// The ASCII characters from `first` to `last`.
fn range(first: char, last: char) -> CharSet {
    std::array::from_fn(|c| (first..=last).contains(&char::from(c as u8)))
}

fn or(a: CharSet, b: CharSet) -> CharSet {
    std::array::from_fn(|c| a[c] || b[c])
}

fn and(a: CharSet, b: CharSet) -> CharSet {
    std::array::from_fn(|c| a[c] && b[c])
}
