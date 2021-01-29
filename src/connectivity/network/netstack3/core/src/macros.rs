// Copyright 2018 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Macros used in Netstack3.

use net_types::ip::{Ipv6Addr, Ipv6SourceAddr};
use net_types::UnicastAddr;

macro_rules! log_unimplemented {
    ($nocrash:expr, $fmt:expr $(,$arg:expr)*) => {{

        #[cfg(feature = "crash_on_unimplemented")]
        unimplemented!($fmt, $($arg),*);

        #[cfg(not(feature = "crash_on_unimplemented"))]
        // Clippy doesn't like blocks explicitly returning ().
        #[allow(clippy::unused_unit)]
        {
            // log doesn't play well with the new macro system; it expects all
            // of its macros to be in scope.
            use ::log::*;
            trace!(concat!("Unimplemented: ", $fmt), $($arg),*);
            $nocrash
        }
    }};

    ($nocrash:expr, $fmt:expr $(,$arg:expr)*,) =>{
        log_unimplemented!($nocrash, $fmt $(,$arg)*)
    };
}

macro_rules! increment_counter {
    ($ctx:ident, $key:expr) => {
        #[cfg(test)]
        $ctx.state_mut().test_counters.increment($key);
    };
}

/// Implement [`TimerContext`] for one ID type in terms of an existing
/// implementation for a different ID type.
///
/// `$outer_timer_id` is an enum where one variant contains an
/// `$inner_timer_id`. `impl_timer_context!` generates an impl of
/// `TimerContext<$inner_timer_id>` for any `C: TimerContext<$outer_timer_id>`.
///
/// An impl of `Into<$outer_timer_id> for `$inner_timer_id` must exist. `$pat`
/// is a pattern of type `$outer_timer_id` that binds the `$inner_timer_id`.
/// `$bound_variable` is the name of the bound `$inner_timer_id` from the
/// pattern. For example, if `$pat` is `OuterTimerId::Inner(id)`, then
/// `$bound_variable` would be `id`. This is required for macro hygiene.
///
/// If an extra first parameter, `$bound`, is provided, then it is added as an
/// extra bound on the `C` context type.
///
/// [`TimerContext`]: crate::context::TimerContext
macro_rules! impl_timer_context {
    ($outer_timer_id:ty, $inner_timer_id:ty, $pat:pat, $bound_variable:ident) => {
        impl<C: crate::context::TimerContext<$outer_timer_id>>
            crate::context::TimerContext<$inner_timer_id> for C
        {
            impl_timer_context!(@inner $inner_timer_id, $pat, $bound_variable);
        }
    };
    ($bound:path, $outer_timer_id:ty, $inner_timer_id:ty, $pat:pat, $bound_variable:ident) => {
        impl<C: $bound + crate::context::TimerContext<$outer_timer_id>>
            crate::context::TimerContext<$inner_timer_id> for C
        {
            impl_timer_context!(@inner $inner_timer_id, $pat, $bound_variable);
        }
    };
    (@inner $inner_timer_id:ty, $pat:pat, $bound_variable:ident) => {
        fn schedule_timer_instant(
            &mut self,
            time: Self::Instant,
            id: $inner_timer_id,
        ) -> Option<Self::Instant> {
            self.schedule_timer_instant(time, id.into())
        }

        fn cancel_timer(&mut self, id: $inner_timer_id) -> Option<Self::Instant> {
            self.cancel_timer(id.into())
        }

        fn cancel_timers_with<F: FnMut(&$inner_timer_id) -> bool>(&mut self, mut f: F) {
            self.cancel_timers_with(|id| match id {
                $pat => f($bound_variable),
                #[allow(unreachable_patterns)]
                _ => false,
            })
        }

        fn scheduled_instant(&self, id: $inner_timer_id) -> Option<Self::Instant> {
            self.scheduled_instant(id.into())
        }
    };
}

/// Declare a benchmark function.
///
/// The function will be named `$name`. If the `benchmark` feature is enabled,
/// it will be annotated with the `#[bench]` attribute, and the provided `$fn`
/// will be invoked with a `&mut test::Bencher` - in other words, a real
/// benchmark. If the `benchmark` feature is disabled, the function will be
/// annotated with the `#[test]` attribute, and the provided `$fn` will be
/// invoked with a `&mut TestBencher`, which has the effect of creating a test
/// that runs the benchmarked function for a single iteration.
///
/// Note that `$fn` doesn't have to be a named function - it can also be an
/// anonymous closure.
#[cfg(test)]
macro_rules! bench {
    ($name:ident, $fn:expr) => {
        #[cfg(feature = "benchmark")]
        #[bench]
        fn $name(b: &mut test::Bencher) {
            $fn(b);
        }

        // TODO(joshlf): Remove the `#[ignore]` once all benchmark tests pass.
        #[cfg(not(feature = "benchmark"))]
        #[test]
        fn $name() {
            $fn(&mut crate::testutil::benchmarks::TestBencher);
        }
    };
}

#[doc(hidden)]
pub(crate) trait TryUnitHelper<T> {
    fn into_option(self) -> Option<T>;
}

impl<T> TryUnitHelper<T> for Option<T> {
    fn into_option(self) -> Option<T> {
        self
    }
}

impl<T, E> TryUnitHelper<T> for Result<T, E> {
    fn into_option(self) -> Option<T> {
        self.ok()
    }
}

impl TryUnitHelper<UnicastAddr<Ipv6Addr>> for Ipv6SourceAddr {
    fn into_option(self) -> Option<UnicastAddr<Ipv6Addr>> {
        self.into()
    }
}

/// Like the `try!` macro, but for functions which return `()`.
///
/// `try_unit!($e)` tries to unwrap `$e` (either as `Result::Ok` or
/// `Option::Some`). If `$e` is instead `Result::Err` or `Option::None`, it
/// returns `()`. If `try_unit!` is invoked as `try_unit!($e, $stmt)` and `$e`
/// is `Result::Err` or `Option::None`, it will also invoke `$stmt` before
/// returning.
macro_rules! try_unit {
    ($e:expr) => {
        match crate::macros::TryUnitHelper::<_>::into_option($e) {
            Some(x) => x,
            None => return,
        }
    };
    ($e:expr, $stmt:stmt) => {
        match crate::macros::TryUnitHelper::<_>::into_option($e) {
            Some(x) => x,
            None => {
                $stmt
                return;
            }
        }
    };
}
