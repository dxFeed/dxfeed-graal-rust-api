//! The event pool of one NetTest instance: one pre-built event per symbol, a table of prices and sizes and the batch
//! fill that stamps the time, the sequence, the price and the size.

use std::time::SystemTime;

use dxfeed_graal::Event;
use dxfeed_graal::event::{TimeAndSale, Trade};

use super::cli::EventKind;
use super::statistics::{NANOS_PER_MILLI, NANOS_PER_SECOND};
use super::symbols::Symbols;

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

/// The step of the instance seeds and the multiplier of the initial state.
const SEED_STEP: u64 = 0x9E37_79B9_7F4A_7C15;
/// The multiplier of the generator and the initial state of a zero one.
const RANDOM_MULTIPLIER: u64 = 0x2545_F491_4F6C_DD1D;

/// The seed of the instance `instance` (from 1).
pub fn instance_seed(instance: u64) -> u64 {
    SEED_STEP.wrapping_mul(instance)
}

/// xorshift64* as the specification defines it, in unsigned 64-bit arithmetic with wrap-around.
struct XorShift64Star {
    state: u64,
}

impl XorShift64Star {
    fn new(seed: u64) -> Self {
        let state = seed.wrapping_mul(SEED_STEP).wrapping_add(RANDOM_MULTIPLIER);
        XorShift64Star {
            state: if state == 0 { RANDOM_MULTIPLIER } else { state },
        }
    }

    fn next(&mut self) -> u64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.wrapping_mul(RANDOM_MULTIPLIER)
    }
}

/// The events of one instance. Not shared: one pool for each generator thread.
pub struct EventPool {
    events: Vec<Event>,
    prices: Vec<f64>,
    sizes: Vec<f64>,
    random: XorShift64Star,
    sequence: u32,
    /// The pool indexes of the events of the batch in the making: the scratch of a fill with `reuse`.
    drawn: Vec<usize>,
}

impl EventPool {
    /// One event per symbol, with the NaN of the defaults in the numeric fields; the instance (from 1) makes the
    /// random source of its symbol and table choices.
    pub fn new(kind: EventKind, symbols: &Symbols, instance: u64) -> Self {
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
            random: XorShift64Star::new(instance_seed(instance)),
            sequence: 0,
            drawn: Vec::new(),
        };
        // Prices of 1.00-1000.99 and integer sizes of 1-1000, two draws a row: the price, then the size.
        for _ in 0..TABLE_SIZE {
            let price = (100 + pool.random.next() % 100_000) as f64 / 100.0;
            let size = (1 + pool.random.next() % 1000) as f64;
            pool.prices.push(price);
            pool.sizes.push(size);
        }
        pool
    }

    /// One draw of an event: the symbol index and the table row.
    fn draw_event(&mut self) -> (usize, usize) {
        let random = self.random.next();
        (
            ((random >> 32) % self.events.len() as u64) as usize,
            ((random >> 8) % TABLE_SIZE as u64) as usize,
        )
    }

    /// The stamp of the next event: the next sequence of the instance (22 bits, wrapping), the time and the row.
    fn next_stamp(&mut self, epoch_nanos: i64, row: usize) -> Stamp {
        let stamp = Stamp {
            packed: pack_time_sequence(epoch_nanos, self.sequence),
            nano_part: (epoch_nanos % NANOS_PER_MILLI) as i32,
            price: self.prices[row],
            size: self.sizes[row],
        };
        self.sequence = self.sequence.wrapping_add(1);
        stamp
    }

    /// Fills the batch with `count` events of random symbols of the pool at the time in nanoseconds since the Unix
    /// epoch: each event gets the time, the nano part, the next sequence of the instance, a price and a size from the
    /// table.
    ///
    /// Without `reuse` the batch is emptied and each event is a new copy of the pool event, stamped on its own; the
    /// pool stays unchanged. With `reuse` the pool event of the drawn symbol is stamped in place, so a symbol drawn
    /// twice appears twice with the later stamp, and the slots of the batch are overwritten with those events.
    pub fn fill_at(&mut self, batch: &mut Vec<Event>, count: usize, reuse: bool, epoch_nanos: i64) {
        if self.events.is_empty() {
            batch.clear();
            return;
        }
        if reuse {
            self.drawn.clear();
            for _ in 0..count {
                let (index, row) = self.draw_event();
                let stamp = self.next_stamp(epoch_nanos, row);
                apply_stamp(&mut self.events[index], &stamp);
                self.drawn.push(index);
            }
            batch.truncate(count);
            for (item, &index) in self.drawn.iter().enumerate() {
                match batch.get_mut(item) {
                    Some(slot) => overwrite(slot, &self.events[index]),
                    None => batch.push(self.events[index].clone()),
                }
            }
        } else {
            batch.clear();
            batch.reserve(count);
            for _ in 0..count {
                let (index, row) = self.draw_event();
                let stamp = self.next_stamp(epoch_nanos, row);
                let mut event = self.events[index].clone();
                apply_stamp(&mut event, &stamp);
                batch.push(event);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::net_test::kit::{kit_hex, kit_json};
    use crate::net_test::symbols::generate_symbols;

    fn symbols(total: i64) -> Symbols {
        Arc::new(generate_symbols(total))
    }

    fn time_millis(packed: i64) -> i64 {
        (packed >> 32) * 1000 + ((packed >> 22) & 0x3FF)
    }

    fn trade(event: &Event) -> &Trade {
        let Event::Trade(trade) = event else {
            panic!("{event:?}");
        };
        trade
    }

    /// 2026-01-01T00:00:01.234567891Z.
    const NANOS: i64 = 1_767_225_601 * NANOS_PER_SECOND + 234_567_891;
    /// The epoch of the first batch in the kit.
    const KIT_EPOCH: i64 = 1_763_380_800_123_456_789;

    #[test]
    fn the_pool_seeds_states_and_draws_equal_the_kit() {
        let vectors = kit_json("pool-vectors.json");
        let instances = vectors["instances"].as_array().unwrap();
        assert_eq!(instances.len(), 2);
        for entry in instances {
            let instance = entry["instance"].as_u64().unwrap();
            let seed = instance_seed(instance);
            assert_eq!(seed, kit_hex(&entry["seed"]), "the seed of {instance}");
            let mut random = XorShift64Star::new(seed);
            assert_eq!(
                random.state,
                kit_hex(&entry["initial_state"]),
                "the state of {instance}"
            );
            for (i, draw) in entry["first_draws"].as_array().unwrap().iter().enumerate() {
                assert_eq!(random.next(), kit_hex(draw), "the draw {i} of {instance}");
            }
        }
    }

    #[test]
    fn the_table_rows_equal_the_kit_and_the_draws_that_build_them() {
        let vectors = kit_json("pool-vectors.json");
        for entry in vectors["instances"].as_array().unwrap() {
            let instance = entry["instance"].as_u64().unwrap();
            let pool = EventPool::new(EventKind::Trade, &symbols(100), instance);
            assert_eq!(pool.prices.len(), TABLE_SIZE);
            assert_eq!(pool.sizes.len(), TABLE_SIZE);
            for row in entry["table_rows"].as_array().unwrap() {
                let index = row["row"].as_u64().unwrap() as usize;
                assert_eq!(
                    pool.prices[index],
                    row["price"].as_f64().unwrap(),
                    "{instance}: price {index}"
                );
                assert_eq!(
                    pool.sizes[index],
                    row["size"].as_f64().unwrap(),
                    "{instance}: size {index}"
                );
            }
            // The first rows from the first draws: the price from the first of a row's two draws, then the size.
            let draws: Vec<u64> = entry["first_draws"].as_array().unwrap().iter().map(kit_hex).collect();
            for row in 0..4 {
                assert_eq!(pool.prices[row], (100 + draws[2 * row] % 100_000) as f64 / 100.0);
                assert_eq!(pool.sizes[row], (1 + draws[2 * row + 1] % 1000) as f64);
            }
        }
    }

    #[test]
    fn the_first_batch_equals_the_kit_with_and_without_reuse() {
        let vectors = kit_json("pool-vectors.json");
        let pool_symbols = symbols(100);
        for entry in vectors["instances"].as_array().unwrap() {
            let instance = entry["instance"].as_u64().unwrap();
            let expected = entry["events"]["first_events"].as_array().unwrap();
            assert_eq!(entry["events"]["pool_size"].as_u64().unwrap(), 100);
            for reuse in [false, true] {
                for kind in [EventKind::Trade, EventKind::TimeAndSale] {
                    let mut pool = EventPool::new(kind, &pool_symbols, instance);
                    let mut batch = Vec::new();
                    pool.fill_at(&mut batch, expected.len(), reuse, KIT_EPOCH);
                    assert_eq!(batch.len(), expected.len());
                    // The draws of the events follow the 2048 draws of the table.
                    let mut random = XorShift64Star::new(instance_seed(instance));
                    (0..2 * TABLE_SIZE).for_each(|_| {
                        random.next();
                    });
                    // Ten events of 100 symbols: with reuse a symbol drawn twice shows the later stamp, so only
                    // the symbols that appear once are compared by sequence.
                    let indexes: Vec<u64> = expected.iter().map(|e| e["symbol_index"].as_u64().unwrap()).collect();
                    for (i, (event, want)) in batch.iter().zip(expected).enumerate() {
                        assert_eq!(random.next(), kit_hex(&want["draw"]), "draw {i} of {instance}");
                        let index = want["symbol_index"].as_u64().unwrap() as usize;
                        let (symbol, packed, nano_part, price, size) = match event {
                            Event::Trade(e) => (&e.symbol, e.time_sequence, e.time_nano_part, e.price, e.size),
                            Event::TimeAndSale(e) => (&e.symbol, e.index, e.time_nano_part, e.price, e.size),
                            other => panic!("{other:?}"),
                        };
                        assert_eq!(*symbol, pool_symbols[index], "symbol {i} of {instance}");
                        let drawn_twice = indexes.iter().filter(|&&other| other as usize == index).count() > 1;
                        if !(reuse && drawn_twice) {
                            assert_eq!(packed, pack_time_sequence(KIT_EPOCH, i as u32), "{i}");
                            assert_eq!(price, want["price"].as_f64().unwrap(), "price {i} of {instance}");
                            assert_eq!(size, want["size"].as_f64().unwrap(), "size {i} of {instance}");
                        }
                        assert_eq!(nano_part, 456_789);
                    }
                }
            }
        }
    }

    #[test]
    fn the_packed_times_equal_the_kit() {
        let vectors = kit_json("pool-vectors.json");
        for entry in vectors["pack_time_sequence"].as_array().unwrap() {
            let epoch = entry["epoch_ns"].as_i64().unwrap();
            let sequence = entry["sequence"].as_u64().unwrap() as u32;
            let packed = pack_time_sequence(epoch, sequence);
            assert_eq!(packed as u64, kit_hex(&entry["packed"]), "{epoch} {sequence}");
            assert_eq!(time_millis(packed), entry["time_ms"].as_i64().unwrap());
            assert_eq!(epoch % NANOS_PER_MILLI, entry["nano_part"].as_i64().unwrap());
        }
    }

    #[test]
    fn the_time_sequence_packing() {
        let packed = pack_time_sequence(NANOS, 5);
        assert_eq!(packed >> 32, 1_767_225_601);
        assert_eq!((packed >> 22) & 0x3FF, 234);
        assert_eq!(packed & 0x3F_FFFF, 5);
        assert_eq!(pack_time_sequence(NANOS, 0x40_0005) & 0x3F_FFFF, 5);
    }

    #[test]
    fn a_zero_state_starts_from_the_multiplier() {
        // The seed that makes the initial state 0: seed * STEP + MULTIPLIER == 0 (mod 2^64).
        let inverse = (1..64).fold(1_u64, |x, _| {
            x.wrapping_mul(2u64.wrapping_sub(SEED_STEP.wrapping_mul(x)))
        });
        assert_eq!(SEED_STEP.wrapping_mul(inverse), 1);
        let seed = RANDOM_MULTIPLIER.wrapping_neg().wrapping_mul(inverse);
        assert_eq!(XorShift64Star::new(seed).state, RANDOM_MULTIPLIER);
    }

    #[test]
    fn a_filled_trade_batch() {
        let pool_symbols = symbols(50);
        let mut pool = EventPool::new(EventKind::Trade, &pool_symbols, 1);
        let mut batch = Vec::new();
        let before = now_epoch_nanos() / NANOS_PER_MILLI;
        pool.fill_at(&mut batch, 20, false, now_epoch_nanos());
        let after = now_epoch_nanos() / NANOS_PER_MILLI;
        assert_eq!(batch.len(), 20);
        for (i, event) in batch.iter().enumerate() {
            let trade = trade(event);
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
    fn the_sequence_continues_across_batches_and_wraps_at_22_bits() {
        let mut pool = EventPool::new(EventKind::Trade, &symbols(10), 3);
        let mut batch = Vec::new();
        let mut sequences = Vec::new();
        for _ in 0..2 {
            pool.fill_at(&mut batch, 3, false, NANOS);
            sequences.extend(batch.iter().map(|event| trade(event).time_sequence & 0x3F_FFFF));
        }
        assert_eq!(sequences, [0, 1, 2, 3, 4, 5]);

        // 2^22 events later the sequence is 0 again; the time part of the packed value is untouched.
        let batch_size = 1 << 16;
        let mut last = 0;
        for _ in 0..(1 << 22) / batch_size - 1 {
            pool.fill_at(&mut batch, batch_size, false, NANOS);
            last = trade(batch.last().unwrap()).time_sequence;
        }
        // Six events were drawn before; the loop drew 2^22 - 2^16 more, so the sequence stands at 2^22 - 2^16 + 6.
        assert_eq!(last & 0x3F_FFFF, (1 << 22) - (1 << 16) + 5);
        pool.fill_at(&mut batch, batch_size, false, NANOS);
        let sequences: Vec<i64> = batch
            .iter()
            .map(|event| trade(event).time_sequence & 0x3F_FFFF)
            .collect();
        // The sequence of the event number 2^22 is 0: it is the event with the index 2^22 - (2^22 - 2^16 + 6).
        let wrap = (1 << 16) - 6;
        assert_eq!(sequences[wrap - 1], i64::from(SEQUENCE_MASK));
        assert_eq!(sequences[wrap], 0);
        assert_eq!(sequences[wrap + 1], 1);
        assert_eq!(
            trade(&batch[wrap]).time_sequence >> 22,
            pack_time_sequence(NANOS, 0) >> 22
        );
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
                let trade = trade(event);
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
    fn the_prices_are_in_range_and_two_instances_differ() {
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
    fn with_reuse_a_symbol_drawn_twice_shares_the_later_stamp() {
        // One symbol: every event of the batch is the same pool event, so all carry the stamp of the last draw.
        let mut pool = EventPool::new(EventKind::Trade, &symbols(1), 1);
        let mut batch = Vec::new();
        pool.fill_at(&mut batch, 5, true, NANOS);
        assert_eq!(batch.len(), 5);
        let last = trade(&batch[4]).clone();
        assert_eq!(last.time_sequence & 0x3F_FFFF, 4);
        for event in &batch {
            let trade = trade(event);
            assert_eq!(
                (trade.time_sequence, trade.price, trade.size),
                (last.time_sequence, last.price, last.size)
            );
        }
        // Without reuse each event keeps its own sequence.
        let mut pool = EventPool::new(EventKind::Trade, &symbols(1), 1);
        pool.fill_at(&mut batch, 5, false, NANOS);
        let sequences: Vec<i64> = batch
            .iter()
            .map(|event| trade(event).time_sequence & 0x3F_FFFF)
            .collect();
        assert_eq!(sequences, [0, 1, 2, 3, 4]);
    }

    #[test]
    fn with_reuse_the_batch_keeps_its_vector_and_the_pool_events_keep_their_defaults() {
        let pool_symbols = symbols(10);
        let mut pool = EventPool::new(EventKind::TimeAndSale, &pool_symbols, 1);
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
            assert!(sale.bid_price.is_nan() && sale.ask_price.is_nan());
        }
        // A smaller batch takes the first slots, a bigger one grows.
        pool.fill_at(&mut batch, 4, true, NANOS);
        assert_eq!(batch.len(), 4);
        pool.fill_at(&mut batch, 12, true, NANOS);
        assert_eq!(batch.len(), 12);
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
        pool.fill_at(&mut batch, 0, true, NANOS);
        assert!(batch.is_empty());
        let mut empty = EventPool::new(EventKind::Trade, &Arc::new(Vec::new()), 1);
        empty.fill_at(&mut batch, 10, true, NANOS);
        assert!(batch.is_empty());
        empty.fill_at(&mut batch, 10, false, NANOS);
        assert!(batch.is_empty());
    }
}
