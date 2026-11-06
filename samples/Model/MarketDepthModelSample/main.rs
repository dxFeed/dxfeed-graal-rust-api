//! Usage: `cargo run --bin MarketDepthModelSample -- [<address> [<symbol> [<sources>]]]`

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dxfeed_graal::event::{IndexedEventSource, Order, OrderSource};
use dxfeed_graal::model::MarketDepthModel;
use dxfeed_graal::{DXEndpoint, system};

/// The number of the orders on each side.
const DEPTH: usize = 19;

/// The line of the screen where the book starts, below the four lines of the header.
const FIRST_LINE: usize = 5;

/// The width of one side of the book.
const SIDE_WIDTH: usize = 39;

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
        draw_header(symbol, sources);
    }

    endpoint.connect(address)?;

    let _ = std::io::stdin().read_line(&mut String::new());

    Ok(())
}

/// Clears the screen and draws the header in its first four lines.
fn draw_header(symbol: &str, sources: &str) {
    let rule = |title: &str, color: &str| format!("{:─^39}", format!(" {title} ")).replace(title, &paint(color, title));
    let lines = [
        format!(" {} {}", paint(BOLD, symbol), paint(DIM, sources)),
        paint(DIM, &format!("{}┬{}", rule("BID", GREEN), rule("ASK", RED))),
        format!(
            " {:<6} {:>12} {:>8} {:>8} {} {:>8} {:>8} {:12} {:<6} ",
            "EX MM",
            "",
            "Size",
            "Price",
            paint(DIM, "│"),
            "Price",
            "Size",
            "",
            "MM EX"
        ),
        paint(DIM, &format!("{0}┼{0}", "─".repeat(SIDE_WIDTH))),
    ];
    let mut screen = String::from(CLEAR_SCREEN);
    for (i, line) in lines.iter().enumerate() {
        screen += &format!("{}{line}{CLEAR_LINE}", move_to(1 + i, 1));
    }
    print!("{screen}");
    let _ = std::io::stdout().flush();
}

/// Draws the book below the header: a line for each order, blank where a side has fewer orders. A bar of each order
/// shows its size against the largest one on the screen.
fn draw_book(buy: &[Order], sell: &[Order]) {
    let max_size = buy.iter().chain(sell).map(|order| order.size).fold(0.0, f64::max);
    let blank = " ".repeat(SIDE_WIDTH);
    let mut screen = String::new();
    for i in 0..DEPTH {
        let bid = buy.get(i).map_or(blank.clone(), |order| {
            let (ex, mm, size, price) = columns(order);
            let bar = format!("{:>12}", bar(order.size, max_size));
            format!(
                " {ex} {mm:<4} {} {size:>8} {} ",
                paint(GREEN, &bar),
                paint(GREEN, &format!("{price:>8}"))
            )
        });
        let ask = sell.get(i).map_or(blank.clone(), |order| {
            let (ex, mm, size, price) = columns(order);
            let bar = format!("{:<12}", bar(order.size, max_size));
            format!(
                " {} {size:>8} {} {mm:<4} {ex} ",
                paint(RED, &format!("{price:>8}")),
                paint(RED, &bar)
            )
        });
        screen += &format!(
            "{}{bid}{}{ask}{CLEAR_LINE}",
            move_to(FIRST_LINE + i, 1),
            paint(DIM, "│")
        );
    }
    print!("{screen}");
    // The standard output of Rust is line-buffered, and the book has no line ends.
    let _ = std::io::stdout().flush();
}

/// The exchange, the market maker, the size and the price of an order, as the book shows them.
fn columns(order: &Order) -> (char, &str, String, String) {
    let ex = Some(order.exchange_code())
        .filter(|c| c.is_ascii_graphic())
        .unwrap_or(' ');
    let mm = order.market_maker.as_deref().unwrap_or("");
    (ex, mm, to_string(order.size), to_string(order.price))
}

/// A bar of up to 12 blocks for `size` against `max_size`; an order of any size has a block at least.
fn bar(size: f64, max_size: f64) -> String {
    let blocks = if size > 0.0 {
        (size / max_size * 12.0).ceil().clamp(1.0, 12.0) as usize
    } else {
        0
    };
    "█".repeat(blocks)
}

/// `text` in the color or style `code`.
fn paint(code: &str, text: &str) -> String {
    format!("{code}{text}{RESET}")
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

// The escape sequences of the terminal (VT100) and its colors (SGR).
const CLEAR_SCREEN: &str = "\x1b[2J";
const CLEAR_LINE: &str = "\x1b[K";
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";

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
