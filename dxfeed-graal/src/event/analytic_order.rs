//! Analytic order events.

use dxfeed_graal_sys as sys;

use super::format::{JavaDouble, Quoted};

order_base_fields! {
    [order_base.order_base]
    event_type! {
        head {
            /// An [`Order`](super::Order) with the analytics of its iceberg part (`AnalyticOrder`).
            AnalyticOrder, AnalyticOrderRef, sys::dxfg_analytic_order_t, sys::DXFG_EVENT_ANALYTIC_ORDER,
            [order_base.order_base.market_event]
        }
        fields {
            /// The identifier of the market maker or of another aggregate of the order; `None` if unknown.
            market_maker: string [order_base.market_maker] = None,
            /// The size of the visible part of the iceberg; NaN if unknown.
            iceberg_peak_size: f64 [iceberg_peak_size] = f64::NAN,
            /// The size of the hidden part of the iceberg; NaN if unknown.
            iceberg_hidden_size: f64 [iceberg_hidden_size] = f64::NAN,
            /// The executed size of the iceberg; NaN if unknown.
            iceberg_executed_size: f64 [iceberg_executed_size] = f64::NAN,
            /// The iceberg flags, packed as the SDK keeps them: the iceberg type in bits 0 and 1.
            iceberg_flags: i32 [iceberg_flags] = 0,
        }
        derived {
            /// The type of the iceberg.
            #[doc(alias = "getIcebergType")]
            fn iceberg_type(iceberg_flags) -> super::IcebergType {
                super::IcebergType::from_code(super::flags::ICEBERG_TYPE.get32(iceberg_flags))
            }
        }
    }
}

impl AnalyticOrder {
    /// Changes the type of the iceberg.
    #[doc(alias = "setIcebergType")]
    pub fn set_iceberg_type(&mut self, iceberg_type: super::IcebergType) {
        self.iceberg_flags = super::flags::ICEBERG_TYPE.set32(self.iceberg_flags, iceberg_type.get_code());
    }
}

/// The text of Java's `AnalyticOrder.toString`, e.g. `AnalyticOrder{AAPL, eventTime=0, source=DEFAULT, ...}`.
impl std::fmt::Display for AnalyticOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AnalyticOrder{")?;
        self.fmt_order_base(f)?;
        write!(
            f,
            ", marketMaker={}, icebergPeakSize={}, icebergHiddenSize={}, icebergExecutedSize={}, icebergType={}}}",
            Quoted(self.market_maker.as_deref()),
            JavaDouble(self.iceberg_peak_size),
            JavaDouble(self.iceberg_hidden_size),
            JavaDouble(self.iceberg_executed_size),
            self.iceberg_type(),
        )
    }
}
