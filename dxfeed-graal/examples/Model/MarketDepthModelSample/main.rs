//! Usage: `cargo run --example MarketDepthModelSample`

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dxfeed_graal::event::{IndexedEventSource, Order, OrderSource};
use dxfeed_graal::model::MarketDepthModel;
use dxfeed_graal::{DXEndpoint, system};

// The portals::legacy functions of the C++ sample.
use legacy as otp;

fn draw_header() {
    println!("{:=^78}", "");
    println!("{:^38}||{:^38}", "BID", "ASK");
    println!(
        "{0:^2}|{1:^13}|{2:^4}|{3:^8}|{4:^7}||{0:^2}|{1:^13}|{2:^4}|{3:^8}|{4:^7}",
        "EX", "Source", "MM", "Price", "Size"
    );
    println!("{:-^78}", "");
}

fn render(buy: &[Order], sell: &[Order], depth: usize) {
    for i in 0..depth {
        if i < buy.len() {
            print!(
                "{}",
                otp::move_to(5 + i, 0)
                    + &format!(
                        "{0:^2}|{1:^13}|{2:^4}|{3:^8}|{4:^7}",
                        encode_char(buy[i].exchange_code()),
                        get_source(&buy[i]),
                        buy[i].market_maker.as_deref().unwrap_or(""),
                        to_string(buy[i].price),
                        to_string(buy[i].size)
                    )
            );
        }

        print!("{}", otp::move_to(5 + i, 39) + "||");

        if i < sell.len() {
            print!(
                "{}",
                otp::move_to(5 + i, 41)
                    + &format!(
                        "{0:^2}|{1:^13}|{2:^4}|{3:^8}|{4:^7}",
                        encode_char(sell[i].exchange_code()),
                        get_source(&sell[i]),
                        sell[i].market_maker.as_deref().unwrap_or(""),
                        to_string(sell[i].price),
                        to_string(sell[i].size)
                    )
            );
        }
    }

    // Unlike std::cout of C++, stdout of Rust is line-buffered.
    let _ = std::io::stdout().flush();
}

fn main() -> dxfeed_graal::Result<()> {
    otp::VirtualTerminal::enable();
    print!("{}", otp::hide_cursor());

    // The cursor comes back on the normal exit and on an error.
    let result = run();
    print!("{}", otp::show_cursor());
    let _ = std::io::stdout().flush();
    result
}

fn run() -> dxfeed_graal::Result<()> {
    let address = "demo.dxfeed.com:7300";
    let symbol = "AAPL";
    let sources = "NTV";
    // let sources = "ntv";
    // let sources = "AGGREGATE_ASK,AGGREGATE_BID";
    let depth = 19_usize;

    // Disable QD logging.
    system::set_property("log.className", "com.devexperts.logging.InterceptableLogging")?;
    system::set_property("log.level", "OFF")?;
    system::set_property("err.level", "OFF")?;

    let io_mtx = Arc::new(Mutex::new(()));

    let endpoint = DXEndpoint::create()?;
    let listener_io_mtx = Arc::clone(&io_mtx);
    let _model = MarketDepthModel::<Order>::new_builder()
        .with_feed(&endpoint.get_feed())
        .with_listener(move |buy, sell| {
            let _lock = listener_io_mtx.lock().unwrap();
            render(buy, sell, depth);
        })
        .with_sources(parse_event_sources(sources)?)
        .with_symbol(symbol)
        .with_depth_limit(depth)
        .with_aggregation_period(Duration::from_millis(30))
        .build()?;

    {
        let _lock = io_mtx.lock().unwrap();
        draw_header();
    }

    endpoint.connect(address)?;

    let _ = std::io::stdin().read_line(&mut String::new());

    Ok(())
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

/// The source of an order.
fn get_source(order: &Order) -> IndexedEventSource {
    IndexedEventSource::from_id(order.source()).unwrap_or_default()
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

/// The functions of `portals::legacy`: VT escape sequences of the terminal.
mod legacy {
    pub struct VirtualTerminal;

    impl VirtualTerminal {
        /// Enables VT interpretation on the standard output of the process: it is a no-op out of Windows.
        #[cfg(windows)]
        pub fn enable() -> bool {
            use std::ffi::c_void;

            type Handle = *mut c_void;

            const STD_OUTPUT_HANDLE: u32 = -11_i32 as u32;
            const ENABLE_PROCESSED_OUTPUT: u32 = 0x0001;
            const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

            unsafe extern "system" {
                fn GetStdHandle(std_handle: u32) -> Handle;
                fn GetConsoleMode(console_handle: Handle, mode: *mut u32) -> i32;
                fn SetConsoleMode(console_handle: Handle, mode: u32) -> i32;
            }

            // SAFETY: the functions of kernel32 get a handle of the process and a valid pointer to a local.
            unsafe {
                let handle = GetStdHandle(STD_OUTPUT_HANDLE);
                let mut mode = 0;

                if handle.is_null() || handle as isize == -1 || GetConsoleMode(handle, &mut mode) == 0 {
                    return false;
                }

                SetConsoleMode(
                    handle,
                    mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
                ) != 0
            }
        }

        /// Enables VT interpretation on the standard output of the process: it is a no-op out of Windows.
        #[cfg(not(windows))]
        pub fn enable() -> bool {
            true
        }
    }

    /// The cursor-position sequence `ESC[line;columnH`.
    pub fn move_to(line: usize, column: usize) -> String {
        format!("\x1b[{line};{column}H")
    }

    /// The cursor-hide sequence.
    pub fn hide_cursor() -> String {
        "\x1b[?25l".to_string()
    }

    /// The cursor-show sequence.
    pub fn show_cursor() -> String {
        "\x1b[?25h".to_string()
    }
}
