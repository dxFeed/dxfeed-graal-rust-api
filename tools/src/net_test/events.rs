//! The event pool of one NetTest instance: one pre-built event per symbol, a table of prices and sizes and the batch
//! fill that stamps the time, the sequence, the price and the size.

use std::time::SystemTime;

use dxfeed_graal::Event;
use dxfeed_graal::event::{TimeAndSale, Trade};

use super::options::EventKind;
use super::symbols::Symbols;

const NANOS_PER_SECOND: i64 = 1_000_000_000;
const NANOS_PER_MILLI: i64 = 1_000_000;

/// The rows of the price and size table.
pub const TABLE_SIZE: usize = 1024;
/// The bits of the sequence in the packed time.
pub const SEQUENCE_MASK: u32 = (1 << 22) - 1;

/// Packs the time and the sequence as `Trade.time_sequence` and `TimeAndSale.index` do: the seconds in the higher 32
/// bits, the milliseconds in bits 22 to 31, the sequence in bits 0 to 21.
pub fn pack_time_sequence(epoch_nanos: i64, sequence: u32) -> i64 {
    let seconds = epoch_nanos / NANOS_PER_SECOND;
    let millis = (epoch_nanos / NANOS_PER_MILLI) % 1000;
    (((seconds as u64) << 32) | ((millis as u64) << 22) | u64::from(sequence & SEQUENCE_MASK)) as i64
}

/// The time of the system clock in nanoseconds since the Unix epoch.
pub fn now_epoch_nanos() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |time| time.as_nanos() as i64)
}

/// What a fill writes into an event.
struct Stamp {
    packed: i64,
    nano_part: i32,
    price: f64,
    size: f64,
}

fn apply_stamp(event: &mut Event, stamp: &Stamp) {
    match event {
        Event::Trade(trade) => {
            trade.time_sequence = stamp.packed;
            trade.time_nano_part = stamp.nano_part;
            trade.price = stamp.price;
            trade.size = stamp.size;
        }
        Event::TimeAndSale(sale) => {
            sale.index = stamp.packed;
            sale.time_nano_part = stamp.nano_part;
            sale.price = stamp.price;
            sale.size = stamp.size;
        }
        _ => {}
    }
}

/// Overwrites `slot` with `source`, reusing the capacity of the strings of the slot.
fn overwrite(slot: &mut Event, source: &Event) {
    match (slot, source) {
        (Event::Trade(slot), Event::Trade(source)) => {
            let mut symbol = std::mem::take(&mut slot.symbol);
            symbol.clone_from(&source.symbol);
            *slot = Trade { symbol, ..*source };
        }
        (Event::TimeAndSale(slot), Event::TimeAndSale(source)) => {
            let mut symbol = std::mem::take(&mut slot.symbol);
            symbol.clone_from(&source.symbol);
            let mut conditions = slot.exchange_sale_conditions.take();
            conditions.clone_from(&source.exchange_sale_conditions);
            let mut buyer = slot.buyer.take();
            buyer.clone_from(&source.buyer);
            let mut seller = slot.seller.take();
            seller.clone_from(&source.seller);
            *slot = TimeAndSale {
                symbol,
                exchange_sale_conditions: conditions,
                buyer,
                seller,
                ..*source
            };
        }
        (slot, source) => slot.clone_from(source),
    }
}

/// The events of one instance. Not shared: one pool for each generator thread.
pub struct EventPool {
    events: Vec<Event>,
    prices: Vec<f64>,
    sizes: Vec<f64>,
    random: u64,
    sequence: u32,
}

impl EventPool {
    /// One event per symbol; the seed makes the symbol choice and the price and size table of the instance.
    pub fn new(kind: EventKind, symbols: &Symbols, seed: u64) -> Self {
        let mut random = seed
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(0x2545_F491_4F6C_DD1D);
        if random == 0 {
            random = 0x2545_F491_4F6C_DD1D;
        }
        let mut pool = EventPool {
            events: symbols
                .iter()
                .map(|symbol| match kind {
                    EventKind::Trade => Event::Trade(Trade::new(symbol)),
                    EventKind::TimeAndSale => Event::TimeAndSale(TimeAndSale::new(symbol)),
                })
                .collect(),
            prices: Vec::with_capacity(TABLE_SIZE),
            sizes: Vec::with_capacity(TABLE_SIZE),
            random,
            sequence: 0,
        };
        // Prices of 1.00-1000.99 and integer sizes of 1-1000.
        for _ in 0..TABLE_SIZE {
            let price = (100 + pool.next_random() % 100_000) as f64 / 100.0;
            let size = (1 + pool.next_random() % 1000) as f64;
            pool.prices.push(price);
            pool.sizes.push(size);
        }
        pool
    }

    /// xorshift64*: cheap and good enough to pick symbols and table rows.
    fn next_random(&mut self) -> u64 {
        self.random ^= self.random >> 12;
        self.random ^= self.random << 25;
        self.random ^= self.random >> 27;
        self.random.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Fills the batch with `count` events of random symbols of the pool at the time in nanoseconds since the Unix
    /// epoch: each event gets the time, the nano part, the next sequence of the instance (22 bits, wrapping), a price
    /// and a size from the table. With `reuse` the slots of the batch are overwritten in place; otherwise the batch is
    /// emptied and gets fresh clones. The pool stays unchanged.
    pub fn fill_at(&mut self, batch: &mut Vec<Event>, count: usize, reuse: bool, epoch_nanos: i64) {
        if reuse {
            batch.truncate(count);
        } else {
            batch.clear();
            batch.reserve(count);
        }
        let pool_size = self.events.len() as u64;
        if pool_size == 0 {
            batch.clear();
            return;
        }
        let nano_part = (epoch_nanos % NANOS_PER_MILLI) as i32;
        for item in 0..count {
            let random = self.next_random();
            let source = &self.events[((random >> 32) % pool_size) as usize];
            let row = ((random >> 8) % TABLE_SIZE as u64) as usize;
            let stamp = Stamp {
                packed: pack_time_sequence(epoch_nanos, self.sequence),
                nano_part,
                price: self.prices[row],
                size: self.sizes[row],
            };
            self.sequence = self.sequence.wrapping_add(1);
            if reuse && item < batch.len() {
                overwrite(&mut batch[item], source);
            } else {
                batch.push(source.clone());
            }
            apply_stamp(&mut batch[item], &stamp);
        }
    }

    /// The same at the current time.
    pub fn fill(&mut self, batch: &mut Vec<Event>, count: usize, reuse: bool) {
        self.fill_at(batch, count, reuse, now_epoch_nanos());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::net_test::symbols::generate_symbols;

    fn symbols(total: i64) -> Symbols {
        Arc::new(generate_symbols(total))
    }

    fn time_millis(packed: i64) -> i64 {
        (packed >> 32) * 1000 + ((packed >> 22) & 0x3FF)
    }

    /// 2026-01-01T00:00:01.234567891Z.
    const NANOS: i64 = 1_767_225_601 * NANOS_PER_SECOND + 234_567_891;

    #[test]
    fn the_time_sequence_packing() {
        let packed = pack_time_sequence(NANOS, 5);
        assert_eq!(packed >> 32, 1_767_225_601);
        assert_eq!((packed >> 22) & 0x3FF, 234);
        assert_eq!(packed & 0x3F_FFFF, 5);
        assert_eq!(pack_time_sequence(NANOS, 0x40_0005) & 0x3F_FFFF, 5);
    }

    #[test]
    fn a_filled_trade_batch() {
        let pool_symbols = symbols(50);
        let mut pool = EventPool::new(EventKind::Trade, &pool_symbols, 1);
        let mut batch = Vec::new();
        let before = now_epoch_nanos() / NANOS_PER_MILLI;
        pool.fill(&mut batch, 20, false);
        let after = now_epoch_nanos() / NANOS_PER_MILLI;
        assert_eq!(batch.len(), 20);
        for (i, event) in batch.iter().enumerate() {
            let Event::Trade(trade) = event else {
                panic!("{event:?}");
            };
            assert!(pool_symbols.contains(&trade.symbol));
            assert!((before..=after + 2000).contains(&time_millis(trade.time_sequence)));
            assert_eq!(trade.time_sequence & 0x3F_FFFF, i as i64);
            assert!((0..1_000_000).contains(&trade.time_nano_part));
            assert!(pool.prices.contains(&trade.price));
            assert!(pool.sizes.contains(&trade.size));
            assert!(trade.change.is_nan() && trade.day_volume.is_nan() && trade.day_turnover.is_nan());
        }
    }

    #[test]
    fn a_filled_time_and_sale_batch() {
        let pool_symbols = symbols(50);
        let mut pool = EventPool::new(EventKind::TimeAndSale, &pool_symbols, 1);
        let mut batch = Vec::new();
        pool.fill_at(&mut batch, 20, false, NANOS);
        assert_eq!(batch.len(), 20);
        for event in &batch {
            let Event::TimeAndSale(sale) = event else {
                panic!("{event:?}");
            };
            assert!(pool_symbols.contains(&sale.symbol));
            assert_eq!(sale.index >> 22, pack_time_sequence(NANOS, 0) >> 22);
            assert_eq!(sale.time_nano_part, 567_891);
            assert!(pool.prices.contains(&sale.price));
            assert!(pool.sizes.contains(&sale.size));
            assert!(sale.bid_price.is_nan() && sale.ask_price.is_nan());
        }
    }

    #[test]
    fn the_fill_at_a_given_time_packs_it_and_the_sequence_wraps_at_22_bits() {
        let mut pool = EventPool::new(EventKind::Trade, &symbols(10), 3);
        pool.sequence = SEQUENCE_MASK - 1;
        let mut batch = Vec::new();
        pool.fill_at(&mut batch, 4, false, NANOS);
        let sequences: Vec<i64> = batch
            .iter()
            .map(|event| {
                let Event::Trade(trade) = event else { panic!() };
                assert_eq!(trade.time_sequence >> 22, pack_time_sequence(NANOS, 0) >> 22);
                assert_eq!(trade.time_nano_part, 567_891);
                trade.time_sequence & 0x3F_FFFF
            })
            .collect();
        assert_eq!(
            sequences,
            [i64::from(SEQUENCE_MASK) - 1, i64::from(SEQUENCE_MASK), 0, 1]
        );
    }

    #[test]
    fn the_prices_are_in_range_and_two_seeds_differ() {
        let pool_symbols = symbols(10);
        let first = EventPool::new(EventKind::Trade, &pool_symbols, 1);
        let same = EventPool::new(EventKind::Trade, &pool_symbols, 1);
        let second = EventPool::new(EventKind::Trade, &pool_symbols, 2);
        assert_eq!(first.prices, same.prices);
        assert_eq!(first.sizes, same.sizes);
        assert_ne!(first.prices, second.prices);
        assert!(first.prices.iter().all(|price| (1.0..=1000.99).contains(price)));
        assert!(
            first
                .sizes
                .iter()
                .all(|size| (1.0..=1000.0).contains(size) && size.fract() == 0.0)
        );
    }

    #[test]
    fn with_reuse_the_batch_keeps_its_vector_and_the_pool_stays_unchanged() {
        let pool_symbols = symbols(10);
        let mut pool = EventPool::new(EventKind::TimeAndSale, &pool_symbols, 1);
        let original = pool.events.clone();
        let mut batch = Vec::new();
        pool.fill_at(&mut batch, 10, true, NANOS);
        let address = batch.as_ptr();
        let capacity = batch.capacity();
        pool.fill_at(&mut batch, 10, true, NANOS + NANOS_PER_SECOND);
        assert_eq!(batch.as_ptr(), address);
        assert_eq!(batch.capacity(), capacity);
        assert_eq!(batch.len(), 10);
        for event in &batch {
            let Event::TimeAndSale(sale) = event else { panic!() };
            assert!(pool_symbols.contains(&sale.symbol));
            assert_eq!(sale.index >> 32, NANOS / NANOS_PER_SECOND + 1);
        }
        // The pool events are never stamped, NaN included.
        assert_eq!(format!("{:?}", pool.events), format!("{original:?}"));
    }

    #[test]
    fn without_reuse_the_pool_stays_unchanged() {
        let pool_symbols = symbols(10);
        let mut pool = EventPool::new(EventKind::Trade, &pool_symbols, 1);
        let original = format!("{:?}", pool.events);
        let mut batch = Vec::new();
        pool.fill_at(&mut batch, 10, false, NANOS);
        assert_eq!(batch.len(), 10);
        drop(batch);
        assert_eq!(format!("{:?}", pool.events), original);
    }

    #[test]
    fn an_empty_request_and_an_empty_pool_give_empty_batches() {
        let mut pool = EventPool::new(EventKind::Trade, &symbols(10), 1);
        let mut batch = Vec::new();
        pool.fill_at(&mut batch, 0, false, NANOS);
        assert!(batch.is_empty());
        let mut empty = EventPool::new(EventKind::Trade, &Arc::new(Vec::new()), 1);
        empty.fill_at(&mut batch, 10, true, NANOS);
        assert!(batch.is_empty());
    }
}
