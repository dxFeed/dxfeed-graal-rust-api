//! The transactional model of one source of indexed events (`IndexedEventTxModel`) and its subscription controller.

use std::fmt;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use dxfeed_graal_sys as sys;

use crate::error::{Error, Result};
use crate::event::{EventKind, Events, IndexedEvent, IndexedEventSource};
use crate::executor::Executor;
use crate::feed::DXFeed;
use crate::handle::Handle;
use crate::isolate::{self, call};
use crate::listener::{EventTxClosure, EventTxListener};
use crate::subscription::{check_aggregation_period, check_events_batch_limit};
use crate::subscription_controller::SubscriptionController;
use crate::symbol::{Symbol, SymbolArena};
use crate::time_period::TimePeriod;

/// A model of the events of one symbol and one source that passes them to its listener in whole transactions and
/// snapshots (`IndexedEventTxModel`). Build it with [`IndexedEventTxModel::new_builder`]; it needs a symbol.
///
/// Unlike [`IndexedTxModel`](super::IndexedTxModel), it takes one source, and its feed and event delivery are changed
/// through its [`get_subscription_controller`](Self::get_subscription_controller).
///
/// Dropping it closes it and releases its listener, whose closure is not called afterwards; dropping it from another
/// thread therefore waits for a running call of the listener, so do not hold a lock its closure takes while dropping
/// it. It keeps the endpoints of the feeds it is attached to open.
///
/// ```no_run
/// use dxfeed_graal::DXFeed;
/// use dxfeed_graal::event::{Order, OrderSource};
/// use dxfeed_graal::model::IndexedEventTxModel;
///
/// # fn main() -> dxfeed_graal::Result<()> {
/// let _model = IndexedEventTxModel::<Order>::new_builder()
///     .with_feed(DXFeed::get_instance()?)
///     .with_symbol("AAPL")
///     .with_source(OrderSource::NTV)
///     .with_listener(|orders, is_snapshot| {
///         println!("{}: {} orders", if is_snapshot { "Snapshot" } else { "Update" }, orders.count());
///     })
///     .build()?;
/// # Ok(())
/// # }
/// ```
pub struct IndexedEventTxModel<E: IndexedEvent + EventKind> {
    handle: Handle<sys::dxfg_v4_indexed_event_tx_model_t>,
    /// In a mutex, so that the model is `Sync`; only `Drop` takes it.
    listener: Mutex<Option<EventTxListener>>,
    _executor: Option<Arc<Handle<sys::dxfg_v4_executor_t>>>,
    /// The feeds the model is attached to; dropped last, as they may close their endpoints.
    feeds: Mutex<Vec<DXFeed>>,
    _event: PhantomData<fn() -> E>,
}

impl<E: IndexedEvent + EventKind> IndexedEventTxModel<E> {
    /// A builder of a model of the events of `E` (`newBuilder`).
    #[doc(alias = "newBuilder")]
    pub fn new_builder() -> IndexedEventTxModelBuilder<E> {
        IndexedEventTxModelBuilder {
            feed: None,
            symbol: None,
            source: None,
            events_batch_limit: None,
            aggregation_period: None,
            executor: None,
            listener: None,
            _event: PhantomData,
        }
    }

    /// The symbol of the events (`getSymbol`).
    #[doc(alias = "getSymbol")]
    pub fn get_symbol(&self) -> Result<String> {
        let mut symbol: *const std::os::raw::c_char = std::ptr::null();
        let m = self.handle.as_ptr();
        // SAFETY: the model is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_IndexedEventTxModel_getSymbol(t, m, &mut symbol) })?;
        // SAFETY: the SDK wrote NULL or a string that is ours to release.
        Ok(unsafe { crate::ipf::profile::take_string(symbol) })
    }

    /// The source of the events (`getSource`).
    #[doc(alias = "getSource")]
    pub fn get_source(&self) -> Result<IndexedEventSource> {
        let mut source: *mut sys::dxfg_v4_indexed_event_source_t = std::ptr::null_mut();
        let m = self.handle.as_ptr();
        // SAFETY: the model is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_IndexedEventTxModel_getSource(t, m, &mut source) })?;
        // SAFETY: the SDK wrote NULL or a live source, which is ours to release.
        let id = unsafe { source.as_ref() }.map(|source| source.id);
        if !source.is_null() {
            // SAFETY: the source is ours and nothing uses it afterwards.
            crate::error_manager::report_release(
                "IndexedEventSource",
                call(|t| unsafe { sys::dxfg_v4_IndexedEventSource_release(t, source) }),
            );
        }
        Ok(id
            .and_then(|id| IndexedEventSource::from_id(id).ok())
            .unwrap_or_default())
    }

    /// The controller of the subscription of the model: it attaches the model to feeds and detaches it, and changes
    /// how the events are delivered (`getSubscriptionController`).
    #[doc(alias = "getSubscriptionController")]
    pub fn get_subscription_controller(&self) -> Result<SubscriptionController<'_>> {
        let m = self.handle.as_ptr();
        // SAFETY: the model is live and the out-pointer is valid for writes.
        let handle =
            Handle::create(|t, out| unsafe { sys::dxfg_v4_IndexedEventTxModel_getSubscriptionController(t, m, out) })?;
        Ok(SubscriptionController::new(handle, &self.feeds, None))
    }

    /// Whether the model is closed (`isClosed`).
    #[doc(alias = "isClosed")]
    pub fn is_closed(&self) -> Result<bool> {
        let mut closed = 0;
        let m = self.handle.as_ptr();
        // SAFETY: the model is live and the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_IndexedEventTxModel_isClosed(t, m, &mut closed) })?;
        Ok(closed != 0)
    }

    /// Detaches the model from the feeds for good and lets go of them; closing it again does nothing (`close`).
    /// Dropping the model closes it too.
    pub fn close(&self) -> Result<()> {
        let mut feeds = crate::lock(&self.feeds);
        self.close_model()?;
        feeds.clear();
        Ok(())
    }

    fn close_model(&self) -> Result<()> {
        let m = self.handle.as_ptr();
        // SAFETY: the model is live.
        call(|t| unsafe { sys::dxfg_v4_IndexedEventTxModel_close(t, m) })
    }
}

impl<E: IndexedEvent + EventKind> Drop for IndexedEventTxModel<E> {
    fn drop(&mut self) {
        if isolate::exiting() {
            // No SDK calls while the process exits; the listener leaks, as the SDK may still call its trampoline.
            return;
        }
        crate::error_manager::report_release("IndexedEventTxModel", self.close_model());
        let listener = self.listener.get_mut().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(listener) = listener {
            listener.release();
        }
    }
}

impl<E: IndexedEvent + EventKind> fmt::Debug for IndexedEventTxModel<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexedEventTxModel")
            .field("event_type", &E::TYPE)
            .finish_non_exhaustive()
    }
}

/// The builder of an [`IndexedEventTxModel`] (`IndexedEventTxModel.Builder`).
pub struct IndexedEventTxModelBuilder<E: IndexedEvent + EventKind> {
    feed: Option<DXFeed>,
    symbol: Option<Symbol>,
    source: Option<IndexedEventSource>,
    events_batch_limit: Option<i32>,
    aggregation_period: Option<TimePeriod>,
    executor: Option<Arc<Handle<sys::dxfg_v4_executor_t>>>,
    listener: Option<Box<EventTxClosure>>,
    _event: PhantomData<fn() -> E>,
}

impl<E: IndexedEvent + EventKind> IndexedEventTxModelBuilder<E> {
    /// The feed the model attaches to when it is built (`withFeed`); the model keeps it.
    #[doc(alias = "withFeed")]
    pub fn with_feed(mut self, feed: &DXFeed) -> Self {
        self.feed = Some(feed.clone());
        self
    }

    /// The symbol of the events (`withSymbol`): a string, or a [`Symbol::indexed`] symbol, which sets the source
    /// too, as [`with_source`](Self::with_source) does.
    #[doc(alias = "withSymbol")]
    pub fn with_symbol(mut self, symbol: impl Into<Symbol>) -> Self {
        let symbol = symbol.into();
        if let Symbol::IndexedEventSubscription { source, .. } = &symbol {
            self.source = Some(*source);
        }
        self.symbol = Some(symbol);
        self
    }

    /// The source of the events; by default, `DEFAULT` (`withSource`). The last of this and an indexed symbol of
    /// [`with_symbol`](Self::with_symbol) wins.
    #[doc(alias = "withSource")]
    pub fn with_source(mut self, source: IndexedEventSource) -> Self {
        self.source = Some(source);
        self
    }

    /// The most events of one call of the listener; a transaction that is larger comes whole
    /// (`withEventsBatchLimit`).
    #[doc(alias = "withEventsBatchLimit")]
    pub fn with_events_batch_limit(mut self, limit: i32) -> Self {
        self.events_batch_limit = Some(limit);
        self
    }

    /// How long the model gathers the events before it calls the listener (`withAggregationPeriod`); at most
    /// [`DXFeedSubscription::AGGREGATION_PERIOD_MAX_VALUE`](crate::DXFeedSubscription::AGGREGATION_PERIOD_MAX_VALUE).
    #[doc(alias = "withAggregationPeriod")]
    pub fn with_aggregation_period(mut self, period: TimePeriod) -> Self {
        self.aggregation_period = Some(period);
        self
    }

    /// The executor that calls the listener; by default, the executor of the feed (`withExecutor`).
    #[doc(alias = "withExecutor")]
    pub fn with_executor(mut self, executor: &Executor) -> Self {
        self.executor = Some(executor.handle.clone());
        self
    }

    /// The listener of the transactions (`withListener`). It gets the events of one or more whole transactions and
    /// whether they are a snapshot, which replaces the events the listener got before.
    ///
    /// The events are valid during the call only: copy what you keep with `to_owned()`. A panic in `f` is caught and
    /// goes to [`ErrorManager`](crate::ErrorManager).
    #[doc(alias = "withListener")]
    pub fn with_listener(mut self, mut f: impl FnMut(Events<'_, E>, bool) + Send + 'static) -> Self {
        self.listener = Some(Box::new(move |events: Events<'_>, is_snapshot| {
            f(events.typed(), is_snapshot)
        }));
        self
    }

    /// Builds the model; it attaches to the feed of [`with_feed`](Self::with_feed), if any (`build`).
    ///
    /// Returns an error if the symbol or the listener is not set.
    pub fn build(self) -> Result<IndexedEventTxModel<E>> {
        if self.listener.is_none() {
            return Err(Error::InvalidArgument("the model has no listener".into()));
        }
        // SAFETY: the out-pointer is valid for writes.
        let builder =
            Handle::create(|t, out| unsafe { sys::dxfg_v4_IndexedEventTxModel_newBuilder(t, E::TYPE.to_sys(), out) })?;
        let b = builder.as_ptr();
        let mut arena = SymbolArena::new();
        if let Some(feed) = &self.feed {
            let f = feed.as_ptr();
            // SAFETY: the builder and the feed are live.
            call(|t| unsafe { sys::dxfg_v4_IndexedEventTxModel_Builder_withFeed(t, b, f) })?;
        }
        let Some(symbol) = &self.symbol else {
            return Err(Error::InvalidArgument("the model has no symbol".into()));
        };
        if let Symbol::IndexedEventSubscription { .. } = symbol {
            let symbol = arena.symbol(symbol, false)?;
            // SAFETY: the builder is live and the symbol lives in the arena during the call.
            call(|t| unsafe { sys::dxfg_v4_IndexedEventTxModel_Builder_withSymbol(t, b, symbol) })?;
        } else {
            // The SDK takes other symbols as strings only.
            let symbol = arena.string(&symbol.to_string(), "the symbol")?;
            // SAFETY: the builder is live and the string lives in the arena during the call.
            call(|t| unsafe { sys::dxfg_v4_IndexedEventTxModel_Builder_withStringSymbol(t, b, symbol) })?;
        }
        // Java needs a source.
        let source = arena.source(self.source.unwrap_or_default());
        // SAFETY: the builder is live and the source lives in the arena during the call.
        call(|t| unsafe { sys::dxfg_v4_IndexedEventTxModel_Builder_withSource(t, b, source) })?;
        if let Some(limit) = self.events_batch_limit {
            check_events_batch_limit(limit)?;
            // SAFETY: the builder is live.
            call(|t| unsafe { sys::dxfg_v4_IndexedEventTxModel_Builder_withEventsBatchLimit(t, b, limit) })?;
        }
        if let Some(period) = self.aggregation_period {
            check_aggregation_period(Some(period))?;
            let period = period.to_sdk()?;
            let p = period.as_ptr();
            // SAFETY: the builder and the period are live.
            call(|t| unsafe { sys::dxfg_v4_IndexedEventTxModel_Builder_withAggregationPeriod(t, b, p) })?;
        }
        if let Some(executor) = &self.executor {
            let e = executor.as_ptr();
            // SAFETY: the builder and the executor are live.
            call(|t| unsafe { sys::dxfg_v4_IndexedEventTxModel_Builder_withExecutor(t, b, e) })?;
        }

        let listener = self.listener.map(EventTxListener::new).transpose()?;
        let built = (|| {
            if let Some(listener) = &listener {
                let l = listener.as_ptr();
                // SAFETY: the builder and the listener are live.
                call(|t| unsafe { sys::dxfg_v4_IndexedEventTxModel_Builder_withListener(t, b, l) })?;
            }
            // SAFETY: the builder is live and the out-pointer is valid for writes.
            Handle::create(|t, out| unsafe { sys::dxfg_v4_IndexedEventTxModel_Builder_build(t, b, out) })
        })();
        match built {
            Ok(handle) => Ok(IndexedEventTxModel {
                handle,
                listener: Mutex::new(listener),
                _executor: self.executor,
                feeds: Mutex::new(self.feed.into_iter().collect()),
                _event: PhantomData,
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

impl<E: IndexedEvent + EventKind> fmt::Debug for IndexedEventTxModelBuilder<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexedEventTxModelBuilder")
            .field("event_type", &E::TYPE)
            .field("symbol", &self.symbol)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}
