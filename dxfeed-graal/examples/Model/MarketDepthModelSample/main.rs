//! Usage: `cargo run --example MarketDepthModelSample -- [<address> [<symbol> [<sources>]]]`

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dxfeed_graal::event::{IndexedEventSource, Order, OrderSource};
use dxfeed_graal::model::MarketDepthModel;
use dxfeed_graal::{DXEndpoint, system};

/// The number of the price levels on each side.
const DEPTH: usize = 19;

/// The line of the screen where the book starts, below the four lines of the header.
const FIRST_LINE: usize = 5;

/// The width of one side of the book.
const SIDE_WIDTH: usize = 38;

fn print_usage() {
    println!(
        r#"
Usage:
MarketDepthModelSample [<address> [<symbol> [<sources>]]]

Where:
    address - The address to connect to retrieve data (remote host or local tape file), demo.dxfeed.com:7300 by
              default.
    symbol  - The symbol to show the order book of, AAPL by default.
    sources - Is comma-separated list of order sources, NTV by default (e.g. "ntv" or
              "AGGREGATE_ASK,AGGREGATE_BID").

Press Enter to stop.

Examples:
    MarketDepthModelSample
    MarketDepthModelSample demo.dxfeed.com:7300 IBM AGGREGATE_ASK,AGGREGATE_BID"#
    );
}

fn main() -> dxfeed_graal::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 4 || args.iter().skip(1).any(|arg| arg == "-h" || arg == "--help") {
        print_usage();
        return Ok(());
    }
    let address = args.get(1).map_or("demo.dxfeed.com:7300", String::as_str);
    let symbol = args.get(2).map_or("AAPL", String::as_str);
    let sources = args.get(3).map_or("NTV", String::as_str);

    enable_virtual_terminal();
    print!("{HIDE_CURSOR}");

    // The cursor comes back below the book on the normal exit and on an error.
    let result = run(address, symbol, sources);
    print!("{}{SHOW_CURSOR}", move_to(FIRST_LINE + DEPTH, 1));
    let _ = std::io::stdout().flush();
    result
}

fn run(address: &str, symbol: &str, sources: &str) -> dxfeed_graal::Result<()> {
    // Disable QD logging.
    system::set_property("log.className", "com.devexperts.logging.InterceptableLogging")?;
    system::set_property("log.level", "OFF")?;
    system::set_property("err.level", "OFF")?;

    let screen = Arc::new(Mutex::new(()));

    let endpoint = DXEndpoint::create()?;
    let listener_screen = Arc::clone(&screen);
    let _model = MarketDepthModel::<Order>::new_builder()
        .with_feed(&endpoint.get_feed())
        .with_listener(move |buy, sell| {
            let _lock = listener_screen.lock().unwrap();
            draw_book(buy, sell);
        })
        .with_sources(parse_event_sources(sources)?)
        .with_symbol(symbol)
        .with_depth_limit(DEPTH)
        .with_aggregation_period(Duration::from_millis(30))
        .build()?;

    // The screen is cleared after the start of the SDK, which may print warnings of the JVM.
    {
        let _lock = screen.lock().unwrap();
        draw_header(symbol);
    }

    endpoint.connect(address)?;

    let _ = std::io::stdin().read_line(&mut String::new());

    Ok(())
}

/// Clears the screen and draws the header in its first four lines.
fn draw_header(symbol: &str) {
    let lines = [
        format!("{:=^78}", format!(" {symbol} ")),
        format!("{:^38}||{:^38}", "BID", "ASK"),
        format!(
            "{0:^2}|{1:^13}|{2:^4}|{3:^8}|{4:^7}||{0:^2}|{1:^13}|{2:^4}|{3:^8}|{4:^7}",
            "EX", "Source", "MM", "Price", "Size"
        ),
        format!("{:-^78}", ""),
    ];
    let mut screen = String::from(CLEAR_SCREEN);
    for (i, line) in lines.iter().enumerate() {
        screen += &format!("{}{line}{CLEAR_LINE}", move_to(1 + i, 1));
    }
    print!("{screen}");
    let _ = std::io::stdout().flush();
}

/// Draws the book below the header: a line for each price level, blank where a side has fewer levels.
fn draw_book(buy: &[Order], sell: &[Order]) {
    let mut screen = String::new();
    for i in 0..DEPTH {
        let bid = buy.get(i).map(format_order).unwrap_or_default();
        let ask = sell.get(i).map(format_order).unwrap_or_default();
        screen += &format!("{}{bid:SIDE_WIDTH$}||{ask}{CLEAR_LINE}", move_to(FIRST_LINE + i, 1));
    }
    print!("{screen}");
    // The standard output of Rust is line-buffered, and the book has no line ends.
    let _ = std::io::stdout().flush();
}

fn format_order(order: &Order) -> String {
    format!(
        "{0:^2}|{1:^13}|{2:^4}|{3:^8}|{4:^7}",
        encode_char(order.exchange_code()),
        order.source(),
        order.market_maker.as_deref().unwrap_or(""),
        to_string(order.price),
        to_string(order.size)
    )
}

/// The order sources of a comma-separated list of names, such as `NTV` or `AGGREGATE_ASK,AGGREGATE_BID`; the
/// default source for an empty list.
fn parse_event_sources(sources: &str) -> dxfeed_graal::Result<Vec<IndexedEventSource>> {
    if sources.trim().is_empty() {
        return Ok(vec![IndexedEventSource::DEFAULT]);
    }

    sources
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(OrderSource::value_of)
        .collect()
}

/// The text of `dxfcpp::encodeChar`: a printable ASCII character as is, `\0`, or `\uXXXX`.
fn encode_char(c: char) -> String {
    match c {
        ' '..='~' => c.to_string(),
        '\0' => "\\0".to_string(),
        _ => format!("\\u{:04x}", c as u32),
    }
}

/// The text of `dxfcpp::toString(double)`, which is the text of `Double.toString` in Java. The same for usual prices
/// and sizes; values from 1e7 and below 1e-3 are printed without an exponent here.
fn to_string(d: f64) -> String {
    if d.is_nan() {
        "NaN".to_string()
    } else if d.is_infinite() {
        if d > 0.0 { "Infinity" } else { "-Infinity" }.to_string()
    } else {
        format!("{d:?}")
    }
}

// The escape sequences of the terminal (VT100).
const CLEAR_SCREEN: &str = "\x1b[2J";
const CLEAR_LINE: &str = "\x1b[K";
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";

/// Moves the cursor to `line` and `column`, both from 1.
fn move_to(line: usize, column: usize) -> String {
    format!("\x1b[{line};{column}H")
}

/// Makes the Windows console interpret the escape sequences; other terminals always do.
#[cfg(windows)]
fn enable_virtual_terminal() {
    use std::ffi::c_void;

    const STD_OUTPUT_HANDLE: u32 = -11_i32 as u32;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    unsafe extern "system" {
        fn GetStdHandle(std_handle: u32) -> *mut c_void;
        fn GetConsoleMode(console_handle: *mut c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(console_handle: *mut c_void, mode: u32) -> i32;
    }

    // SAFETY: the functions of kernel32 get the handle of the process's output and a pointer to a local; a failure,
    // such as an output that is no console, leaves the mode as it is.
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(handle, &mut mode) != 0 {
            SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
        }
    }
}

#[cfg(not(windows))]
fn enable_virtual_terminal() {}
