//! Feed filters: named categories of QD filter expressions that a subscription or an event passes (experimental).
//!
//! A [`DXFeedFilter`] is built from categories, each a name and a QD filter expression such as `IBM,AAPL`, `A*`,
//! `range-A-M-` or `ipf[file.ipf,update=10m]`. It tells whether a subscription, an event type with a symbol, or an
//! event passes any category, and which category is the first one that it passes, in the order the categories were
//! added. A filter does not change; a dynamic one, such as an `ipf[...]` expression with an `update` period, gets new
//! versions, which its [`DXFeedFilterTracker`] returns and announces to its listeners.
//!
//! The module needs the cargo feature `experimental`: the Java API (`com.dxfeed.api.DXFeedFilter`) and the SDK
//! functions are experimental and may change in a minor release.
//!
//! ```
//! use dxfeed_graal::event::{EventType, Quote};
//! use dxfeed_graal::feed_filter::DXFeedFilter;
//! use dxfeed_graal::DXEndpoint;
//!
//! let endpoint = DXEndpoint::create()?;
//! let filter = DXFeedFilter::new_builder(&endpoint)
//!     .with_category("big", "IBM,AAPL")
//!     .with_category("rest", "*")
//!     .build()?;
//! assert_eq!(filter.get_category(EventType::Quote, "IBM")?.as_deref(), Some("big"));
//! assert_eq!(filter.get_event_category(&Quote::new("MSFT").into())?.as_deref(), Some("rest"));
//! # Ok::<(), dxfeed_graal::Error>(())
//! ```

use std::fmt;
use std::slice;

use dxfeed_graal_sys as sys;

use crate::endpoint::DXEndpoint;
use crate::error::Result;
use crate::event::sdk_event::needs_candle_symbol;
use crate::event::{Event, EventType, SysEvents};
use crate::handle::Handle;
use crate::isolate::{self, call, out_optional_string};
use crate::listener::{FilterListener, FilterListeners};
use crate::subscription::ListenerId;
use crate::symbol::{Symbol, SymbolArena};

/// Builds a [`DXFeedFilter`]; see [`DXFeedFilter::new_builder`].
///
/// The categories are checked and passed to the SDK by [`build`](Self::build).
#[derive(Clone)]
#[must_use]
pub struct DXFeedFilterBuilder {
    endpoint: DXEndpoint,
    categories: Vec<(String, String)>,
}

impl DXFeedFilterBuilder {
    /// Adds the category `category_name` of the QD filter expression `filter_expression` (`withCategory`). The
    /// categories are evaluated in the order they are added.
    #[doc(alias = "withCategory")]
    pub fn with_category(mut self, category_name: &str, filter_expression: &str) -> Self {
        self.categories
            .push((category_name.to_owned(), filter_expression.to_owned()));
        self
    }

    /// Adds the categories of `categories` in their order, as [`with_category`](Self::with_category) does for each
    /// (`withCategories`).
    #[doc(alias = "withCategories")]
    pub fn with_categories<K: AsRef<str>, V: AsRef<str>>(
        mut self,
        categories: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        self.categories.extend(
            categories
                .into_iter()
                .map(|(name, expression)| (name.as_ref().to_owned(), expression.as_ref().to_owned())),
        );
        self
    }

    /// The endpoint whose data scheme parses the expressions (`getEndpoint`).
    #[doc(alias = "getEndpoint")]
    pub fn get_endpoint(&self) -> &DXEndpoint {
        &self.endpoint
    }

    /// The categories, names with their expressions, in their order (`getCategories`).
    #[doc(alias = "getCategories")]
    pub fn get_categories(&self) -> &[(String, String)] {
        &self.categories
    }

    /// Builds the filter (`build`).
    ///
    /// No category, an empty or repeated name or an empty expression is [`Error::Java`] of
    /// `java.lang.IllegalArgumentException`, and an expression that QD cannot parse is one of its subclass
    /// `com.devexperts.qd.kit.FilterSyntaxException`; a name or an expression with a NUL byte is
    /// [`Error::InvalidArgument`].
    ///
    /// [`Error::Java`]: crate::Error::Java
    /// [`Error::InvalidArgument`]: crate::Error::InvalidArgument
    pub fn build(&self) -> Result<DXFeedFilter> {
        let mut arena = SymbolArena::new();
        let names = self
            .categories
            .iter()
            .map(|(name, _)| arena.string(name, "the category name"))
            .collect::<Result<Vec<_>>>()?;
        let expressions = self
            .categories
            .iter()
            .map(|(_, expression)| arena.string(expression, "the filter expression"))
            .collect::<Result<Vec<_>>>()?;
        let e = self.endpoint.as_ptr();
        // SAFETY: the endpoint is live; the out-pointer is valid for writes.
        let builder = Handle::create(|t, out| unsafe { sys::dxfg_v4_DXFeedFilter_newBuilder(t, e, out) })?;
        let b = builder.as_ptr();
        let list = |strings: &[*const std::ffi::c_char]| sys::dxfg_v4_string_list {
            size: strings.len() as i32,
            elements: strings.as_ptr().cast_mut(),
        };
        let (names, expressions) = (list(&names), list(&expressions));
        // SAFETY: the builder is live, and the lists and their strings live in this frame; the SDK only reads them.
        call(|t| unsafe { sys::dxfg_v4_DXFeedFilter_Builder_withCategories(t, b, &names, &expressions) })?;
        // SAFETY: the builder is live; the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_DXFeedFilter_Builder_build(t, b, out) })?;
        Ok(DXFeedFilter { handle })
    }
}

impl fmt::Debug for DXFeedFilterBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DXFeedFilterBuilder")
            .field("categories", &self.categories)
            .finish_non_exhaustive()
    }
}

/// A filter of subscriptions and events by named categories of QD filter expressions (`DXFeedFilter`); see the
/// [module](self).
pub struct DXFeedFilter {
    handle: Handle<sys::dxfg_v4_feed_filter_t>,
}

impl DXFeedFilter {
    /// A builder of a filter whose expressions are parsed with the data scheme of `endpoint`, without categories
    /// (`newBuilder`).
    #[doc(alias = "newBuilder")]
    pub fn new_builder(endpoint: &DXEndpoint) -> DXFeedFilterBuilder {
        DXFeedFilterBuilder {
            endpoint: endpoint.clone(),
            categories: Vec::new(),
        }
    }

    /// Whether the subscription to `event_type` and `symbol` passes any category (`accept`).
    pub fn accept(&self, event_type: EventType, symbol: impl Into<Symbol>) -> Result<bool> {
        let (f, mut arena, mut accepted) = (self.handle.as_ptr(), SymbolArena::new(), 0);
        let s = arena.symbol(&symbol.into(), needs_candle_symbol(event_type))?;
        // SAFETY: the filter is live, the symbol lives in the arena; the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_DXFeedFilter_accept(t, f, event_type.to_sys(), s, &mut accepted) })?;
        Ok(accepted != 0)
    }

    /// Whether `event` passes any category (`acceptEvent`).
    #[doc(alias = "acceptEvent")]
    pub fn accept_event(&self, event: &Event) -> Result<bool> {
        let (f, events, mut accepted) = (self.handle.as_ptr(), SysEvents::new(slice::from_ref(event))?, 0);
        let e = events.pointers()[0];
        // SAFETY: the filter is live, the event lives in `events`; the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_DXFeedFilter_acceptEvent(t, f, e, &mut accepted) })?;
        Ok(accepted != 0)
    }

    /// The first category that the subscription to `event_type` and `symbol` passes; `None` if it passes none
    /// (`getCategory`).
    #[doc(alias = "getCategory")]
    pub fn get_category(&self, event_type: EventType, symbol: impl Into<Symbol>) -> Result<Option<String>> {
        let (f, mut arena) = (self.handle.as_ptr(), SymbolArena::new());
        let s = arena.symbol(&symbol.into(), needs_candle_symbol(event_type))?;
        // SAFETY: the filter is live, the symbol lives in the arena; the out-pointer is valid for writes.
        out_optional_string("DXFeedFilter", |t, out| unsafe {
            sys::dxfg_v4_DXFeedFilter_getCategory(t, f, event_type.to_sys(), s, out)
        })
    }

    /// The first category that `event` passes; `None` if it passes none (`getEventCategory`).
    #[doc(alias = "getEventCategory")]
    pub fn get_event_category(&self, event: &Event) -> Result<Option<String>> {
        let (f, events) = (self.handle.as_ptr(), SysEvents::new(slice::from_ref(event))?);
        let e = events.pointers()[0];
        // SAFETY: the filter is live, the event lives in `events`; the out-pointer is valid for writes.
        out_optional_string("DXFeedFilter", |t, out| unsafe {
            sys::dxfg_v4_DXFeedFilter_getEventCategory(t, f, e, out)
        })
    }

    /// Whether the filter gets new versions, which its [tracker](Self::get_tracker) follows (`isDynamic`).
    #[doc(alias = "isDynamic")]
    pub fn is_dynamic(&self) -> Result<bool> {
        let (f, mut dynamic) = (self.handle.as_ptr(), 0);
        // SAFETY: the filter is live; the out-pointer is valid for writes.
        call(|t| unsafe { sys::dxfg_v4_DXFeedFilter_isDynamic(t, f, &mut dynamic) })?;
        Ok(dynamic != 0)
    }

    /// The tracker of the versions of the filter (`getTracker`).
    #[doc(alias = "getTracker")]
    pub fn get_tracker(&self) -> Result<DXFeedFilterTracker> {
        let f = self.handle.as_ptr();
        // SAFETY: the filter is live; the out-pointer is valid for writes.
        let handle = Handle::create(|t, out| unsafe { sys::dxfg_v4_DXFeedFilter_getTracker(t, f, out) })?;
        Ok(DXFeedFilterTracker {
            handle,
            listeners: FilterListeners::new("a listener of this tracker"),
        })
    }
}

impl fmt::Display for DXFeedFilter {
    /// The text of Java's `toString`, which identifies the version of the filter, not its categories.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.handle.to_java_string() {
            Ok(text) => f.write_str(&text),
            Err(error) => write!(f, "DXFeedFilter(<{error}>)"),
        }
    }
}

impl fmt::Debug for DXFeedFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DXFeedFilter").finish_non_exhaustive()
    }
}

/// The tracker of the versions of a [`DXFeedFilter`] (`DXFeedFilterTracker`): its current filter, and listeners
/// called after each update of a dynamic filter.
///
/// The tracker follows the updates only while it has a listener; without one it may return an older version. The
/// listeners added through this value are removed when it is dropped.
pub struct DXFeedFilterTracker {
    handle: Handle<sys::dxfg_v4_feed_filter_tracker_t>,
    listeners: FilterListeners,
}

impl DXFeedFilterTracker {
    /// The latest version of the filter (`getCurrentFilter`).
    #[doc(alias = "getCurrentFilter")]
    pub fn get_current_filter(&self) -> Result<DXFeedFilter> {
        let t = self.handle.as_ptr();
        // SAFETY: the tracker is live; the out-pointer is valid for writes.
        let handle =
            Handle::create(|th, out| unsafe { sys::dxfg_v4_DXFeedFilterTracker_getCurrentFilter(th, t, out) })?;
        Ok(DXFeedFilter { handle })
    }

    /// Adds `listener`, called after each update of the filter on a thread of the SDK that updates it
    /// (`addListener`); [`get_current_filter`](Self::get_current_filter) returns the new version. Fast updates may
    /// come as one call. The tracker of a filter that is not dynamic never calls it.
    #[doc(alias = "addListener")]
    pub fn add_listener(&self, listener: impl FnMut() + Send + 'static) -> Result<ListenerId> {
        let t = self.handle.as_ptr();
        self.listeners.add(FilterListener::new(Box::new(listener))?, |l| {
            // SAFETY: the tracker and the listener are live.
            call(|th| unsafe { sys::dxfg_v4_DXFeedFilterTracker_addListener(th, t, l) })
        })
    }

    /// Removes a listener, waiting for its running call unless called from it (`removeListener`).
    ///
    /// Returns [`Error::InvalidArgument`](crate::Error::InvalidArgument) if the listener was not added through this
    /// value.
    #[doc(alias = "removeListener")]
    pub fn remove_listener(&self, id: ListenerId) -> Result<()> {
        let t = self.handle.as_ptr();
        self.listeners.remove(id, |l| {
            // SAFETY: the tracker and the listener are live.
            call(|th| unsafe { sys::dxfg_v4_DXFeedFilterTracker_removeListener(th, t, l) })
        })
    }
}

impl Drop for DXFeedFilterTracker {
    fn drop(&mut self) {
        if isolate::exiting() {
            // No SDK calls from destructors while the process exits; the listeners' state is leaked.
            return;
        }
        let t = self.handle.as_ptr();
        self.listeners.remove_all("DXFeedFilterTracker", |l| {
            // SAFETY: the tracker and the listener are live.
            call(|th| unsafe { sys::dxfg_v4_DXFeedFilterTracker_removeListener(th, t, l) })
        });
    }
}

impl fmt::Debug for DXFeedFilterTracker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DXFeedFilterTracker").finish_non_exhaustive()
    }
}
