//! Transactional models of indexed and time-series events (`IndexedTxModel`, `TimeSeriesTxModel`): a model subscribes
//! to the events of a symbol and passes them to its listener in whole transactions and snapshots only.
//!
//! ```no_run
//! use dxfeed_graal::DXFeed;
//! use dxfeed_graal::event::{Order, OrderSource};
//! use dxfeed_graal::model::IndexedTxModel;
//!
//! # fn main() -> dxfeed_graal::Result<()> {
//! let _model = IndexedTxModel::<Order>::new_builder()
//!     .with_feed(DXFeed::get_instance()?)
//!     .with_symbol("AAPL")
//!     .with_sources([OrderSource::NTV])
//!     .with_listener(|source, orders, is_snapshot| {
//!         println!("{} from {source}:", if is_snapshot { "Snapshot" } else { "Update" });
//!         for order in orders {
//!             println!("    {order}");
//!         }
//!     })
//!     .build()?;
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result};
use crate::event::{Events, IndexedEvent, IndexedEventSource, TimeSeriesEvent};
use crate::executor::ManualExecutor;
use crate::feed::DXFeed;
use crate::handle::{Handle, SdkHandle};
use crate::isolate::{self, call};
use crate::listener::{TxClosure, TxListener};
use crate::symbol::{Symbol, SymbolArena};

mod indexed_event;
mod market_depth;

pub use indexed_event::{IndexedEventTxModel, IndexedEventTxModelBuilder, SubscriptionController};
pub use market_depth::{MarketDepthModel, MarketDepthModelBuilder};

type Thread = *mut sys::graal_isolatethread_t;
/// A setter of an SDK builder: it writes the builder to use next to its out-pointer.
type With<B, A> = unsafe extern "C" fn(Thread, *mut B, A, *mut *mut B) -> i32;
/// A getter of an SDK model.
type Get<M, V> = unsafe extern "C" fn(Thread, *mut M, *mut V) -> i32;
/// A method of an SDK model with one argument.
type Act<M, A> = unsafe extern "C" fn(Thread, *mut M, A) -> i32;
/// The constructor of an SDK builder.
type NewBuilder<B> = unsafe extern "C" fn(Thread, sys::dxfg_event_clazz_t, *mut *mut B) -> i32;
/// The `build` of an SDK builder.
type Build<B, M> = unsafe extern "C" fn(Thread, *mut B, *mut *mut M) -> i32;

/// The SDK functions that both kinds of models have.
trait Kind {
    type Builder: SdkHandle;
    type Model: SdkHandle;
    const NEW_BUILDER: NewBuilder<Self::Builder>;
    const WITH_BATCH_PROCESSING: With<Self::Builder, i32>;
    const WITH_SNAPSHOT_PROCESSING: With<Self::Builder, i32>;
    const WITH_FEED: With<Self::Builder, *mut sys::dxfg_v4_feed_t>;
    const WITH_SYMBOL: With<Self::Builder, *mut sys::dxfg_v4_symbol_t>;
    const WITH_AGGREGATION_PERIOD: With<Self::Builder, *mut sys::dxfg_v4_time_period_t>;
    const WITH_LISTENER: With<Self::Builder, *mut sys::dxfg_v4_tx_model_listener_t>;
    const WITH_EXECUTOR: With<Self::Builder, *mut sys::dxfg_v4_executor_t>;
    const BUILD: Build<Self::Builder, Self::Model>;
    const IS_BATCH_PROCESSING: Get<Self::Model, i32>;
    const IS_SNAPSHOT_PROCESSING: Get<Self::Model, i32>;
    const ATTACH: Act<Self::Model, *mut sys::dxfg_v4_feed_t>;
    const DETACH: Act<Self::Model, *mut sys::dxfg_v4_feed_t>;
    const SET_AGGREGATION_PERIOD: Act<Self::Model, *mut sys::dxfg_v4_time_period_t>;
    const CLOSE: unsafe extern "C" fn(Thread, *mut Self::Model) -> i32;
}

/// Implements [`Kind`] for `$kind` with the SDK functions of the Java class `$class`.
macro_rules! kind {
    ($kind:ident, $builder:ty, $model:ty, $class:literal, {
        $new_builder:ident, $batch:ident, $snapshot:ident, $feed:ident, $symbol:ident, $period:ident,
        $listener:ident, $executor:ident, $build:ident, $is_batch:ident, $is_snapshot:ident, $attach:ident,
        $detach:ident, $set_period:ident, $close:ident $(,)?
    }) => {
        #[doc = concat!("The SDK functions of `", $class, "`.")]
        struct $kind;

        impl Kind for $kind {
            type Builder = $builder;
            type Model = $model;
            const NEW_BUILDER: NewBuilder<Self::Builder> = sys::$new_builder;
            const WITH_BATCH_PROCESSING: With<Self::Builder, i32> = sys::$batch;
            const WITH_SNAPSHOT_PROCESSING: With<Self::Builder, i32> = sys::$snapshot;
            const WITH_FEED: With<Self::Builder, *mut sys::dxfg_v4_feed_t> = sys::$feed;
            const WITH_SYMBOL: With<Self::Builder, *mut sys::dxfg_v4_symbol_t> = sys::$symbol;
            const WITH_AGGREGATION_PERIOD: With<Self::Builder, *mut sys::dxfg_v4_time_period_t> = sys::$period;
            const WITH_LISTENER: With<Self::Builder, *mut sys::dxfg_v4_tx_model_listener_t> = sys::$listener;
            const WITH_EXECUTOR: With<Self::Builder, *mut sys::dxfg_v4_executor_t> = sys::$executor;
            const BUILD: Build<Self::Builder, Self::Model> = sys::$build;
            const IS_BATCH_PROCESSING: Get<Self::Model, i32> = sys::$is_batch;
            const IS_SNAPSHOT_PROCESSING: Get<Self::Model, i32> = sys::$is_snapshot;
            const ATTACH: Act<Self::Model, *mut sys::dxfg_v4_feed_t> = sys::$attach;
            const DETACH: Act<Self::Model, *mut sys::dxfg_v4_feed_t> = sys::$detach;
            const SET_AGGREGATION_PERIOD: Act<Self::Model, *mut sys::dxfg_v4_time_period_t> = sys::$set_period;
            const CLOSE: unsafe extern "C" fn(Thread, *mut Self::Model) -> i32 = sys::$close;
        }
    };
}

kind!(Indexed, sys::dxfg_v4_indexed_tx_model_builder_t, sys::dxfg_v4_indexed_tx_model_t, "IndexedTxModel", {
    dxfg_v4_IndexedTxModel_newBuilder,
    dxfg_v4_IndexedTxModel_Builder_withBatchProcessing,
    dxfg_v4_IndexedTxModel_Builder_withSnapshotProcessing,
    dxfg_v4_IndexedTxModel_Builder_withFeed,
    dxfg_v4_IndexedTxModel_Builder_withSymbol,
    dxfg_v4_IndexedTxModel_Builder_withAggregationPeriod,
    dxfg_v4_IndexedTxModel_Builder_withListener,
    dxfg_v4_IndexedTxModel_Builder_withExecutor,
    dxfg_v4_IndexedTxModel_Builder_build,
    dxfg_v4_IndexedTxModel_isBatchProcessing,
    dxfg_v4_IndexedTxModel_isSnapshotProcessing,
    dxfg_v4_IndexedTxModel_attach,
    dxfg_v4_IndexedTxModel_detach,
    dxfg_v4_IndexedTxModel_setAggregationPeriod,
    dxfg_v4_IndexedTxModel_close,
});

kind!(TimeSeries, sys::dxfg_v4_time_series_tx_model_builder_t, sys::dxfg_v4_time_series_tx_model_t,
    "TimeSeriesTxModel", {
    dxfg_v4_TimeSeriesTxModel_newBuilder,
    dxfg_v4_TimeSeriesTxModel_Builder_withBatchProcessing,
    dxfg_v4_TimeSeriesTxModel_Builder_withSnapshotProcessing,
    dxfg_v4_TimeSeriesTxModel_Builder_withFeed,
    dxfg_v4_TimeSeriesTxModel_Builder_withSymbol,
    dxfg_v4_TimeSeriesTxModel_Builder_withAggregationPeriod,
    dxfg_v4_TimeSeriesTxModel_Builder_withListener,
    dxfg_v4_TimeSeriesTxModel_Builder_withExecutor,
    dxfg_v4_TimeSeriesTxModel_Builder_build,
    dxfg_v4_TimeSeriesTxModel_isBatchProcessing,
    dxfg_v4_TimeSeriesTxModel_isSnapshotProcessing,
    dxfg_v4_TimeSeriesTxModel_attach,
    dxfg_v4_TimeSeriesTxModel_detach,
    dxfg_v4_TimeSeriesTxModel_setAggregationPeriod,
    dxfg_v4_TimeSeriesTxModel_close,
});

/// The settings that both kinds of builders have; `None` keeps the default of Java.
#[derive(Default)]
struct Settings {
    batch_processing: Option<bool>,
    snapshot_processing: Option<bool>,
    feed: Option<DXFeed>,
    symbol: Option<Symbol>,
    aggregation_period: Option<Duration>,
    executor: Option<Arc<Handle<sys::dxfg_v4_executor_t>>>,
    listener: Option<Box<TxClosure>>,
}

impl Settings {
    /// Builds the model of kind `K` for the events of `event_type`; `more` applies the settings of the kind.
    fn build<K: Kind>(
        self,
        event_type: sys::dxfg_event_clazz_t,
        more: impl FnOnce(Handle<K::Builder>) -> Result<Handle<K::Builder>>,
    ) -> Result<Core<K>> {
        // SAFETY: the out-pointer is valid for writes.
        let mut builder = Handle::create(|t, out| unsafe { K::NEW_BUILDER(t, event_type, out) })?;
        if let Some(on) = self.batch_processing {
            builder = with(builder, K::WITH_BATCH_PROCESSING, i32::from(on))?;
        }
        if let Some(on) = self.snapshot_processing {
            builder = with(builder, K::WITH_SNAPSHOT_PROCESSING, i32::from(on))?;
        }
        if let Some(feed) = &self.feed {
            builder = with(builder, K::WITH_FEED, feed.as_ptr())?;
        }
        if let Some(symbol) = &self.symbol {
            let mut arena = SymbolArena::new();
            let symbol = arena.symbol(symbol, false)?;
            builder = with(builder, K::WITH_SYMBOL, symbol)?;
        }
        if let Some(period) = self.aggregation_period {
            let period = time_period(period)?;
            builder = with(builder, K::WITH_AGGREGATION_PERIOD, period.as_ptr())?;
        }
        if let Some(executor) = &self.executor {
            builder = with(builder, K::WITH_EXECUTOR, executor.as_ptr())?;
        }
        builder = more(builder)?;

        let listener = self.listener.map(TxListener::new).transpose()?;
        let built = (|| {
            if let Some(listener) = &listener {
                builder = with(builder, K::WITH_LISTENER, listener.as_ptr())?;
            }
            let b = builder.as_ptr();
            // SAFETY: the builder is live and the out-pointer is valid for writes.
            Handle::create(|t, out| unsafe { K::BUILD(t, b, out) })
        })();
        match built {
            Ok(handle) => Ok(Core {
                handle,
                listener: Mutex::new(listener),
                _executor: self.executor,
                feeds: Mutex::new(self.feed.into_iter().collect()),
            }),
            Err(e) => {
                if let Some(listener) = listener {
                    listener.release();
                }
                Err(e)
            }
        }
    }
}

/// Applies a setter of an SDK builder and returns the builder to use next; the previous handle is released.
fn with<B: SdkHandle, A>(builder: Handle<B>, set: With<B, A>, value: A) -> Result<Handle<B>> {
    let b = builder.as_ptr();
    // SAFETY: the builder is live, the value is live during the call and the out-pointer is valid for writes.
    Handle::create(|t, out| unsafe { set(t, b, value, out) })
}

/// The Java `TimePeriod` of a duration, in whole milliseconds.
fn time_period(period: Duration) -> Result<Handle<sys::dxfg_v4_time_period_t>> {
    let millis = i64::try_from(period.as_millis())
        .map_err(|_| Error::InvalidArgument(format!("the period {period:?} is too long")))?;
    // SAFETY: the out-pointer is valid for writes.
    Handle::create(|t, out| unsafe { sys::dxfg_v4_TimePeriod_valueOf(t, millis, out) })
}

/// What both kinds of models do, around the SDK model.
struct Core<K: Kind> {
    handle: Handle<K::Model>,
    /// In a mutex, so that the model is `Sync`; only `Drop` takes it.
    listener: Mutex<Option<TxListener>>,
    _executor: Option<Arc<Handle<sys::dxfg_v4_executor_t>>>,
    /// The feeds the model is attached to; dropped last, as they may close their endpoints.
    feeds: Mutex<Vec<DXFeed>>,
}

impl<K: Kind> Core<K> {
    fn flag(&self, get: Get<K::Model, i32>) -> Result<bool> {
        let mut value = 0;
        let m = self.handle.as_ptr();
        // SAFETY: the model is live and the out-pointer is valid for writes.
        call(|t| unsafe { get(t, m, &mut value) })?;
        Ok(value != 0)
    }

    fn attach(&self, feed: &DXFeed) -> Result<()> {
        // The lock keeps the SDK call and the kept feeds in step with a concurrent `detach`.
        let mut feeds = crate::lock(&self.feeds);
        let (m, f) = (self.handle.as_ptr(), feed.as_ptr());
        // SAFETY: the model and the feed are live.
        call(|t| unsafe { K::ATTACH(t, m, f) })?;
        if !feeds.iter().any(|kept| kept.same_as(feed)) {
            feeds.push(feed.clone());
        }
        Ok(())
    }

    fn detach(&self, feed: &DXFeed) -> Result<()> {
        let mut feeds = crate::lock(&self.feeds);
        let (m, f) = (self.handle.as_ptr(), feed.as_ptr());
        // SAFETY: the model and the feed are live.
        call(|t| unsafe { K::DETACH(t, m, f) })?;
        feeds.retain(|kept| !kept.same_as(feed));
        Ok(())
    }

    fn set_aggregation_period(&self, period: Duration) -> Result<()> {
        let period = time_period(period)?;
        let (m, p) = (self.handle.as_ptr(), period.as_ptr());
        // SAFETY: the model and the period are live.
        call(|t| unsafe { K::SET_AGGREGATION_PERIOD(t, m, p) })
    }

    /// Closes the model and lets go of its feeds.
    fn close(&self) -> Result<()> {
        let mut feeds = crate::lock(&self.feeds);
        self.close_model()?;
        feeds.clear();
        Ok(())
    }

    fn close_model(&self) -> Result<()> {
        let m = self.handle.as_ptr();
        // SAFETY: the model is live.
        call(|t| unsafe { K::CLOSE(t, m) })
    }
}

impl<K: Kind> Drop for Core<K> {
    fn drop(&mut self) {
        if isolate::exiting() {
            // No SDK calls while the process exits; the listener leaks, as the SDK may still call its trampoline.
            return;
        }
        // The feeds go last, after the listener: they may close their endpoints.
        let _ = self.close_model();
        let listener = self.listener.get_mut().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(listener) = listener {
            listener.release();
        }
    }
}

/// The setters that both kinds of builders have.
macro_rules! builder_methods {
    () => {
        /// Whether the listener gets all the pending transactions in one call (`true`, the default) or one call per
        /// transaction (`withBatchProcessing`).
        #[doc(alias = "withBatchProcessing")]
        pub fn with_batch_processing(mut self, batch_processing: bool) -> Self {
            self.settings.batch_processing = Some(batch_processing);
            self
        }

        /// Whether a snapshot comes as its resulting state, without the removed events and the events that later
        /// events of the snapshot replace (`true`), or as it comes (`false`, the default) (`withSnapshotProcessing`).
        #[doc(alias = "withSnapshotProcessing")]
        pub fn with_snapshot_processing(mut self, snapshot_processing: bool) -> Self {
            self.settings.snapshot_processing = Some(snapshot_processing);
            self
        }

        /// The feed the model attaches to when it is built; the model keeps it (`withFeed`).
        #[doc(alias = "withFeed")]
        pub fn with_feed(mut self, feed: &DXFeed) -> Self {
            self.settings.feed = Some(feed.clone());
            self
        }

        /// The symbol of the events; it must be set (`withSymbol`).
        #[doc(alias = "withSymbol")]
        pub fn with_symbol(mut self, symbol: impl Into<Symbol>) -> Self {
            self.settings.symbol = Some(symbol.into());
            self
        }

        /// The period over which the events are aggregated, in whole milliseconds; zero, the default, for none
        /// (`withAggregationPeriod`).
        #[doc(alias = "withAggregationPeriod")]
        pub fn with_aggregation_period(mut self, period: Duration) -> Self {
            self.settings.aggregation_period = Some(period);
            self
        }

        /// The executor that calls the listener; by default, the executor of the feed (`withExecutor`).
        #[doc(alias = "withExecutor")]
        pub fn with_executor(mut self, executor: &ManualExecutor) -> Self {
            self.settings.executor = Some(executor.handle.clone());
            self
        }

        /// The listener of the transactions (`withListener`). It gets the source of the events, the events of one
        /// or, with batch processing, several whole transactions, and whether they are a snapshot, which replaces
        /// the events of that source the listener got before.
        ///
        /// The events are valid during the call only: copy what you keep with `to_owned()`. A panic in `f` is
        /// caught and printed to stderr.
        #[doc(alias = "withListener")]
        pub fn with_listener(
            mut self,
            mut f: impl FnMut(IndexedEventSource, Events<'_, E>, bool) + Send + 'static,
        ) -> Self {
            self.settings.listener = Some(Box::new(move |source, events: Events<'_>, is_snapshot| {
                f(source, events.typed(), is_snapshot)
            }));
            self
        }
    };
}

/// The methods that both kinds of models have.
macro_rules! model_methods {
    () => {
        /// Whether the listener gets all the pending transactions in one call (`isBatchProcessing`).
        #[doc(alias = "isBatchProcessing")]
        pub fn is_batch_processing(&self) -> Result<bool> {
            self.core.flag(<Self as HasKind>::Kind::IS_BATCH_PROCESSING)
        }

        /// Whether a snapshot comes as its resulting state (`isSnapshotProcessing`).
        #[doc(alias = "isSnapshotProcessing")]
        pub fn is_snapshot_processing(&self) -> Result<bool> {
            self.core.flag(<Self as HasKind>::Kind::IS_SNAPSHOT_PROCESSING)
        }

        /// Attaches the model to a feed, which it keeps until it is detached (`attach`).
        pub fn attach(&self, feed: &DXFeed) -> Result<()> {
            self.core.attach(feed)
        }

        /// Detaches the model from a feed (`detach`).
        pub fn detach(&self, feed: &DXFeed) -> Result<()> {
            self.core.detach(feed)
        }

        /// Changes the period over which the events are aggregated, in whole milliseconds (`setAggregationPeriod`).
        #[doc(alias = "setAggregationPeriod")]
        pub fn set_aggregation_period(&self, period: Duration) -> Result<()> {
            self.core.set_aggregation_period(period)
        }

        /// Detaches the model from all the feeds for good and lets go of them; closing it again does nothing
        /// (`close`). Dropping the model closes it too.
        pub fn close(&self) -> Result<()> {
            self.core.close()
        }
    };
}

/// The kind of a model type.
trait HasKind {
    type Kind: Kind;
}

/// A model of the indexed events of type `E` of one symbol and some sources (`IndexedTxModel`), created with
/// [`IndexedTxModel::new_builder`]: the listener gets them in whole transactions and snapshots, per source.
///
/// Dropping it closes it and releases its listener, whose closure is not called afterwards; dropping it from another
/// thread therefore waits for a running call of the listener, so do not hold a lock its closure takes while dropping
/// it. It keeps the endpoints of the feeds it is attached to open.
pub struct IndexedTxModel<E: IndexedEvent> {
    core: Core<Indexed>,
    _event: PhantomData<fn() -> E>,
}

impl<E: IndexedEvent> HasKind for IndexedTxModel<E> {
    type Kind = Indexed;
}

impl<E: IndexedEvent> IndexedTxModel<E> {
    /// A builder of a model of the events of type `E` (`newBuilder`).
    #[doc(alias = "newBuilder")]
    pub fn new_builder() -> IndexedTxModelBuilder<E> {
        IndexedTxModelBuilder {
            settings: Settings::default(),
            sources: None,
            _event: PhantomData,
        }
    }

    /// The sources of the events; empty for all the sources (`getSources`).
    #[doc(alias = "getSources")]
    pub fn get_sources(&self) -> Result<Vec<IndexedEventSource>> {
        let mut list: *mut sys::dxfg_v4_indexed_event_source_list = std::ptr::null_mut();
        let m = self.core.handle.as_ptr();
        // SAFETY: the model is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_IndexedTxModel_getSources(t, m, &mut list) })?;
        // SAFETY: the SDK wrote NULL or a list of `size` live sources, which is ours to release.
        let sources = unsafe { list.as_ref() }
            .filter(|list| list.size > 0 && !list.elements.is_null())
            .map(|list| unsafe { std::slice::from_raw_parts(list.elements, list.size as usize) })
            .unwrap_or_default()
            .iter()
            // SAFETY: the elements of a live list are NULL or live sources.
            .filter_map(|&source| unsafe { source.as_ref() })
            .map(|source| IndexedEventSource::from_id(source.id).unwrap_or_default())
            .collect();
        // SAFETY: the list is ours and nothing uses it afterwards.
        let _ = call(|t| unsafe { sys::dxfg_v4_indexed_event_source_list_release(t, list) });
        Ok(sources)
    }

    /// Changes the sources of the events: the model subscribes to the new ones and unsubscribes from the others;
    /// none means all the sources (`setSources`).
    #[doc(alias = "setSources")]
    pub fn set_sources(&self, sources: impl IntoIterator<Item = IndexedEventSource>) -> Result<()> {
        let mut arena = SymbolArena::new();
        let list = arena.source_list(sources);
        let m = self.core.handle.as_ptr();
        // SAFETY: the model is live and the list lives in the arena during the call.
        call(|t| unsafe { sys::dxfg_v4_IndexedTxModel_setSources(t, m, list) })
    }

    model_methods!();
}

impl<E: IndexedEvent> fmt::Debug for IndexedTxModel<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexedTxModel")
            .field("event_type", &E::TYPE)
            .finish_non_exhaustive()
    }
}

/// The builder of an [`IndexedTxModel`] (`IndexedTxModel.Builder`).
pub struct IndexedTxModelBuilder<E: IndexedEvent> {
    settings: Settings,
    sources: Option<Vec<IndexedEventSource>>,
    _event: PhantomData<fn() -> E>,
}

impl<E: IndexedEvent> IndexedTxModelBuilder<E> {
    builder_methods!();

    /// The sources of the events; none, the default, means all the sources (`withSources`).
    #[doc(alias = "withSources")]
    pub fn with_sources(mut self, sources: impl IntoIterator<Item = IndexedEventSource>) -> Self {
        self.sources = Some(sources.into_iter().collect());
        self
    }

    /// Builds the model; it attaches to the feed of [`with_feed`](Self::with_feed), if any (`build`).
    ///
    /// Returns an error if the symbol is not set.
    pub fn build(self) -> Result<IndexedTxModel<E>> {
        let sources = self.sources;
        let core = self
            .settings
            .build::<Indexed>(E::TYPE.to_sys(), |builder| match &sources {
                Some(sources) => {
                    let mut arena = SymbolArena::new();
                    let list = arena.source_list(sources.iter().copied());
                    with(builder, sys::dxfg_v4_IndexedTxModel_Builder_withSources, list)
                }
                None => Ok(builder),
            })?;
        Ok(IndexedTxModel {
            core,
            _event: PhantomData,
        })
    }
}

/// The order of the events of a snapshot of a [`TimeSeriesTxModel`] (`TimeSeriesTxModel.SortOrder`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortOrder {
    /// The order in which the events come (`NONE`).
    None,
    /// Ascending time (`ASCENDING`).
    Ascending,
    /// Descending time (`DESCENDING`).
    Descending,
}

impl SortOrder {
    fn to_sys(self) -> sys::dxfg_v4_TimeSeriesTxModelSortOrder_t {
        match self {
            SortOrder::None => sys::NONE,
            SortOrder::Ascending => sys::ASCENDING,
            SortOrder::Descending => sys::DESCENDING,
        }
    }
}

/// A model of the time-series events of type `E` of one symbol from a time on (`TimeSeriesTxModel`), created with
/// [`TimeSeriesTxModel::new_builder`]: the listener gets them in whole transactions and snapshots.
///
/// Dropping it closes it and releases its listener, whose closure is not called afterwards; dropping it from another
/// thread therefore waits for a running call of the listener, so do not hold a lock its closure takes while dropping
/// it. It keeps the endpoints of the feeds it is attached to open.
pub struct TimeSeriesTxModel<E: TimeSeriesEvent> {
    core: Core<TimeSeries>,
    _event: PhantomData<fn() -> E>,
}

impl<E: TimeSeriesEvent> HasKind for TimeSeriesTxModel<E> {
    type Kind = TimeSeries;
}

impl<E: TimeSeriesEvent> TimeSeriesTxModel<E> {
    /// A builder of a model of the events of type `E` (`newBuilder`).
    #[doc(alias = "newBuilder")]
    pub fn new_builder() -> TimeSeriesTxModelBuilder<E> {
        TimeSeriesTxModelBuilder {
            settings: Settings::default(),
            from_time: None,
            sorting: None,
            ignore_remove_events: None,
            ignore_events_from_past: None,
            confirmation_tick: None,
            _event: PhantomData,
        }
    }

    /// The time the model is subscribed from, in milliseconds since the Unix epoch; `i64::MAX` when it is not
    /// subscribed (`getFromTime`).
    #[doc(alias = "getFromTime")]
    pub fn get_from_time(&self) -> Result<i64> {
        let mut time = 0;
        let m = self.core.handle.as_ptr();
        // SAFETY: the model is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_TimeSeriesTxModel_getFromTime(t, m, &mut time) })?;
        Ok(time)
    }

    /// Subscribes the model from a time on, in milliseconds since the Unix epoch, or unsubscribes it with `i64::MAX`;
    /// the listener gets a new snapshot (`setFromTime`).
    #[doc(alias = "setFromTime")]
    pub fn set_from_time(&self, from_time: i64) -> Result<()> {
        let m = self.core.handle.as_ptr();
        // SAFETY: the model is live.
        call(|t| unsafe { sys::dxfg_v4_TimeSeriesTxModel_setFromTime(t, m, from_time) })
    }

    model_methods!();
}

impl<E: TimeSeriesEvent> fmt::Debug for TimeSeriesTxModel<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TimeSeriesTxModel")
            .field("event_type", &E::TYPE)
            .finish_non_exhaustive()
    }
}

/// The builder of a [`TimeSeriesTxModel`] (`TimeSeriesTxModel.Builder`).
pub struct TimeSeriesTxModelBuilder<E: TimeSeriesEvent> {
    settings: Settings,
    from_time: Option<i64>,
    sorting: Option<SortOrder>,
    ignore_remove_events: Option<bool>,
    ignore_events_from_past: Option<bool>,
    confirmation_tick: Option<bool>,
    _event: PhantomData<fn() -> E>,
}

impl<E: TimeSeriesEvent> TimeSeriesTxModelBuilder<E> {
    builder_methods!();

    /// The time to subscribe from, in milliseconds since the Unix epoch; by default `i64::MAX`, not subscribed
    /// (`withFromTime`).
    #[doc(alias = "withFromTime")]
    pub fn with_from_time(mut self, from_time: i64) -> Self {
        self.from_time = Some(from_time);
        self
    }

    /// The order of the events of a snapshot (`withSorting`).
    #[doc(alias = "withSorting")]
    pub fn with_sorting(mut self, sorting: SortOrder) -> Self {
        self.sorting = Some(sorting);
        self
    }

    /// Whether the removed events are dropped (`true`) or delivered (`false`, the default) (`ignoreRemoveEvents`).
    #[doc(alias = "ignoreRemoveEvents")]
    pub fn ignore_remove_events(mut self, ignore: bool) -> Self {
        self.ignore_remove_events = Some(ignore);
        self
    }

    /// Whether the events with an index not greater than that of the last processed one are dropped (`true`) or
    /// delivered (`false`, the default) (`ignoreEventsFromPast`).
    #[doc(alias = "ignoreEventsFromPast")]
    pub fn ignore_events_from_past(mut self, ignore: bool) -> Self {
        self.ignore_events_from_past = Some(ignore);
        self
    }

    /// Whether a new bar comes after the previous one, which it confirms (`true`), or alone (`false`, the default);
    /// `true` also turns on the snapshot processing and the ascending sorting (`withConfirmationTick`).
    #[doc(alias = "withConfirmationTick")]
    pub fn with_confirmation_tick(mut self, confirmation_tick: bool) -> Self {
        self.confirmation_tick = Some(confirmation_tick);
        self
    }

    /// Builds the model; it attaches to the feed of [`with_feed`](Self::with_feed), if any (`build`).
    ///
    /// Returns an error if the symbol is not set.
    pub fn build(self) -> Result<TimeSeriesTxModel<E>> {
        let TimeSeriesTxModelBuilder {
            settings,
            from_time,
            sorting,
            ignore_remove_events,
            ignore_events_from_past,
            confirmation_tick,
            _event,
        } = self;
        let core = settings.build::<TimeSeries>(E::TYPE.to_sys(), |mut builder| {
            if let Some(from_time) = from_time {
                builder = with(builder, sys::dxfg_v4_TimeSeriesTxModel_Builder_withFromTime, from_time)?;
            }
            if let Some(sorting) = sorting {
                builder = with(
                    builder,
                    sys::dxfg_v4_TimeSeriesTxModel_Builder_withSorting,
                    sorting.to_sys(),
                )?;
            }
            if let Some(ignore) = ignore_remove_events {
                builder = with(
                    builder,
                    sys::dxfg_v4_TimeSeriesTxModel_Builder_ignoreRemoveEvents,
                    i32::from(ignore),
                )?;
            }
            if let Some(ignore) = ignore_events_from_past {
                let set = sys::dxfg_v4_TimeSeriesTxModel_Builder_ignoreEventsFromPast;
                builder = with(builder, set, i32::from(ignore))?;
            }
            if let Some(tick) = confirmation_tick {
                let set = sys::dxfg_v4_TimeSeriesTxModel_Builder_withConfirmationTick;
                builder = with(builder, set, i32::from(tick))?;
            }
            Ok(builder)
        })?;
        Ok(TimeSeriesTxModel {
            core,
            _event: PhantomData,
        })
    }
}
