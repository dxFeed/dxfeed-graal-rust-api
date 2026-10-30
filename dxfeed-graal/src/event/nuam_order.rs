//! Nuam order events.

use dxfeed_graal_sys as sys;

use super::flags::{NUAM_ORDER_TYPE, NUAM_TIME_IN_FORCE};
use super::format::{JavaDouble, Quoted};
use super::{NuamOrderType, NuamTimeInForceType};

order_base_fields! {
    [order_base.order_base]
    event_type! {
        head {
            /// An [`Order`](super::Order) of the Nuam exchange, with its participants and quantities (`NuamOrder`).
            NuamOrder, NuamOrderRef, sys::dxfg_nuam_order_t, sys::DXFG_EVENT_NUAM_ORDER,
            [order_base.order_base.market_event]
        }
        fields {
            /// The identifier of the market maker or of another aggregate of the order; `None` if unknown.
            market_maker: string [order_base.market_maker] = None,
            /// The identifier of the actor.
            actor_id: i32 [actor_id] = 0,
            /// The identifier of the participant.
            participant_id: i32 [participant_id] = 0,
            /// The identifier of the submitter.
            submitter_id: i32 [submitter_id] = 0,
            /// The identifier of the submitter that the order was submitted on behalf of.
            on_behalf_of_submitter_id: i32 [on_behalf_of_submitter_id] = 0,
            /// The identifier the client gave the order; `None` if unknown.
            client_order_id: string [client_order_id] = None,
            /// The account of the customer; `None` if unknown.
            customer_account: string [customer_account] = None,
            /// The information about the customer; `None` if unknown.
            customer_info: string [customer_info] = None,
            /// The information about the exchange; `None` if unknown. Java's `toString` does not print it.
            exchange_info: string [exchange_info] = None,
            /// The data of the time in force, such as the number of days.
            time_in_force_data: i32 [time_in_force_data] = 0,
            /// The identifier of the order book that triggers the order.
            trigger_order_book_id: i32 [trigger_order_book_id] = 0,
            /// The price that triggers the order; NaN if unknown.
            trigger_price: f64 [trigger_price] = f64::NAN,
            /// The type of the session that triggers the order.
            trigger_session_type: i32 [trigger_session_type] = 0,
            /// The quantity of the order; NaN if unknown.
            order_quantity: f64 [order_quantity] = f64::NAN,
            /// The displayed quantity; NaN if unknown.
            display_quantity: f64 [display_quantity] = f64::NAN,
            /// The quantity that refreshes the displayed one; NaN if unknown.
            refresh_quantity: f64 [refresh_quantity] = f64::NAN,
            /// The quantity left to execute; NaN if unknown.
            leaves_quantity: f64 [leaves_quantity] = f64::NAN,
            /// The matched quantity; NaN if unknown.
            matched_quantity: f64 [matched_quantity] = f64::NAN,
            /// The Nuam flags, packed as the SDK keeps them: the time in force in bits 4 to 7, the order type in bits 0
            /// to 3.
            nuam_flags: i32 [nuam_flags] = 0,
        }
        derived {
            /// The time in force of the order.
            #[doc(alias = "getTimeInForce")]
            fn time_in_force(nuam_flags) -> NuamTimeInForceType {
                NuamTimeInForceType::from_code(NUAM_TIME_IN_FORCE.get(nuam_flags.into()))
            }

            /// The type of the order.
            #[doc(alias = "getOrderType")]
            fn order_type(nuam_flags) -> NuamOrderType {
                NuamOrderType::from_code(NUAM_ORDER_TYPE.get(nuam_flags.into()))
            }
        }
    }
}

impl NuamOrder {
    /// Changes the time in force of the order.
    #[doc(alias = "setTimeInForce")]
    pub fn set_time_in_force(&mut self, time_in_force: NuamTimeInForceType) {
        self.nuam_flags = NUAM_TIME_IN_FORCE.set32(self.nuam_flags, time_in_force.code());
    }

    /// Changes the type of the order.
    #[doc(alias = "setOrderType")]
    pub fn set_order_type(&mut self, order_type: NuamOrderType) {
        self.nuam_flags = NUAM_ORDER_TYPE.set32(self.nuam_flags, order_type.code());
    }
}

/// The text of Java's `NuamOrder.toString`, e.g. `NuamOrder{AAPL, eventTime=0, source=DEFAULT, ...}`.
impl std::fmt::Display for NuamOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NuamOrder{")?;
        self.fmt_order_base(f)?;
        write!(
            f,
            ", marketMaker={}, actorId={}, participantId={}, submitterId={}, onBehalfOfSubmitterId={}, \
             clientOrderId={}, customerAccount={}, customerInfo={}, timeInForce={}, timeInForceData={}, \
             triggerOrderBookId={}, triggerPrice={}, triggerSessionType={}, orderType={}, orderQuantity={}, \
             displayQuantity={}, refreshQuantity={}, leavesQuantity={}, matchedQuantity={}}}",
            Quoted(self.market_maker.as_deref()),
            self.actor_id,
            self.participant_id,
            self.submitter_id,
            self.on_behalf_of_submitter_id,
            Quoted(self.client_order_id.as_deref()),
            Quoted(self.customer_account.as_deref()),
            Quoted(self.customer_info.as_deref()),
            self.time_in_force(),
            self.time_in_force_data,
            self.trigger_order_book_id,
            JavaDouble(self.trigger_price),
            self.trigger_session_type,
            self.order_type(),
            JavaDouble(self.order_quantity),
            JavaDouble(self.display_quantity),
            JavaDouble(self.refresh_quantity),
            JavaDouble(self.leaves_quantity),
            JavaDouble(self.matched_quantity),
        )
    }
}
