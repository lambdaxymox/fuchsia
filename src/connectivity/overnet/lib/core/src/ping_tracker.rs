// Copyright 2019 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Schedule pings when they need to be scheduled, provide an estimation of round trip time

use crate::future_help::{Observable, Observer, PollMutex};
use crate::runtime::{maybe_wait_until, Task};
use anyhow::{format_err, Error};
use futures::{
    future::{poll_fn, Either},
    lock::{Mutex, MutexGuard},
    prelude::*,
    ready,
};
use std::collections::{HashMap, VecDeque};
use std::convert::TryInto;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

const MIN_PING_SPACING: Duration = Duration::from_millis(100);
const MAX_PING_SPACING: Duration = Duration::from_secs(20);
const MAX_SAMPLE_AGE: Duration = Duration::from_secs(2 * 60);
const MAX_PING_AGE: Duration = Duration::from_secs(15);

/// A pong record includes an id and the amount of time taken to schedule the pong
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pong {
    /// The requestors ping id
    pub id: u64,
    /// The queue time (in microseconds) before responding
    pub queue_time: u64,
}

#[derive(Debug)]
struct Sample {
    when: Instant,
    rtt_us: i64,
}

struct State {
    round_trip_time: Observable<Option<Duration>>,
    send_ping: bool,
    send_pong: Option<(u64, Instant)>,
    on_send_ping_pong: Option<Waker>,
    sent_ping: Option<(u64, Instant)>,
    on_sent_ping: Option<Waker>,
    received_pong: Option<(Pong, Instant)>,
    on_received_pong: Option<Waker>,
    closed: bool,
    on_closed: Option<Waker>,
    next_ping_id: u64,
}

struct OnSentPing<'a>(PollMutex<'a, State>);
impl<'a> Future for OnSentPing<'a> {
    type Output = Option<(MutexGuard<'a, State>, u64, Instant)>;
    fn poll(mut self: Pin<&mut Self>, ctx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let mut inner = ready!(self.0.poll(ctx));
        match inner.sent_ping.take() {
            Some(r) => Poll::Ready(Some((inner, r.0, r.1))),
            None => {
                if inner.closed {
                    Poll::Ready(None)
                } else {
                    inner.on_sent_ping = Some(ctx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }
}

struct OnReceivedPong<'a>(PollMutex<'a, State>);
impl<'a> Future for OnReceivedPong<'a> {
    type Output = Option<(MutexGuard<'a, State>, Pong, Instant)>;
    fn poll(mut self: Pin<&mut Self>, ctx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let mut inner = ready!(self.0.poll(ctx));
        match inner.received_pong.take() {
            Some(r) => Poll::Ready(Some((inner, r.0, r.1))),
            None => {
                if inner.closed {
                    Poll::Ready(None)
                } else {
                    inner.on_received_pong = Some(ctx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }
}

async fn ping_pong(state: Arc<Mutex<State>>) -> Result<(), Error> {
    struct Stats {
        samples: VecDeque<Sample>,
        variance: i64,
        sent_ping_map: HashMap<u64, Instant>,
        sent_ping_list: VecDeque<(u64, Instant)>,
        ping_spacing: Duration,
        last_ping_sent: Instant,
        timeout: Option<Instant>,
        wake_on_change_timeout: Option<Waker>,
        closed: bool,
        published_mean: i64,
    };

    impl Stats {
        async fn recalculate(&mut self, state: MutexGuard<'_, State>) -> Result<(), Error> {
            let variance_before = self.variance;
            let mut mean = 0i64;
            let mut total = 0i64;
            let n = self.samples.len() as i64;
            if n == 0i64 {
                self.variance = 0;
            } else {
                for Sample { rtt_us, .. } in self.samples.iter() {
                    total = total
                        .checked_add(*rtt_us)
                        .ok_or_else(|| format_err!("Overflow calculating mean"))?;
                }
                mean = total / n;
                if n == 1i64 {
                    self.variance = 0;
                } else {
                    let mut numer = 0;
                    for Sample { rtt_us, .. } in self.samples.iter() {
                        numer += square(rtt_us - mean)
                            .ok_or_else(|| format_err!("Overflow calculating variance"))?;
                    }
                    self.variance = numer / (n - 1i64);
                }
            }
            // Publish state
            if self.variance > variance_before {
                self.ping_spacing /= 2;
                if self.ping_spacing < MIN_PING_SPACING {
                    self.ping_spacing = MIN_PING_SPACING;
                }
            } else if self.variance < variance_before {
                self.ping_spacing = self.ping_spacing * 5 / 4;
                if self.ping_spacing > MAX_PING_SPACING {
                    self.ping_spacing = MAX_PING_SPACING;
                }
            }
            let new_round_trip_time = mean > 0
                && (self.published_mean <= 0
                    || (self.published_mean - mean).abs() > self.published_mean / 10);
            if new_round_trip_time {
                self.published_mean = mean;
                state.round_trip_time.push(Some(Duration::from_micros(mean as u64))).await;
            }
            if !state.send_ping {
                self.timeout = Some(self.last_ping_sent + self.ping_spacing);
                self.wake_on_change_timeout.take().map(|w| w.wake());
            }
            Ok(())
        }

        fn garbage_collect(&mut self) {
            let now = Instant::now();
            if let Some(epoch) = now.checked_sub(MAX_SAMPLE_AGE) {
                while self.samples.len() > 3 && self.samples[0].when < epoch {
                    self.samples.pop_front();
                }
            }
            if let Some(epoch) = now.checked_sub(MAX_PING_AGE) {
                while self.sent_ping_list.len() > 1 && self.sent_ping_list[0].1 < epoch {
                    self.sent_ping_map.remove(&self.sent_ping_list.pop_front().unwrap().0);
                }
            }
        }
    }

    let stats = Mutex::new(Stats {
        samples: VecDeque::new(),
        variance: 0i64,
        sent_ping_map: HashMap::new(),
        sent_ping_list: VecDeque::new(),
        ping_spacing: Duration::from_millis(100),
        last_ping_sent: Instant::now(),
        timeout: None,
        wake_on_change_timeout: None,
        closed: false,
        published_mean: 0i64,
    });

    let state_ref: &Mutex<State> = &*state;
    let stats_ref: &Mutex<Stats> = &stats;

    futures::future::try_join3(
        async move {
            while let Some((state, id, when)) = OnSentPing(PollMutex::new(state_ref)).await {
                let mut stats = stats_ref.lock().await;
                stats.last_ping_sent = when;
                stats.sent_ping_map.insert(id, when);
                stats.sent_ping_list.push_back((id, when));
                stats.sent_ping_map.insert(id, when);
                stats.recalculate(state).await?;
            }
            // Propagate closure to timer task.
            let mut stats = stats_ref.lock().await;
            stats.closed = true;
            stats.wake_on_change_timeout.take().map(|w| w.wake());
            Ok::<_, Error>(())
        },
        async move {
            while let Some((state, Pong { id, queue_time }, when)) =
                OnReceivedPong(PollMutex::new(state_ref)).await
            {
                let mut stats = stats_ref.lock().await;
                let now = Instant::now();
                if let Some(ping_sent) = stats.sent_ping_map.get(&id) {
                    if let Some(rtt) =
                        (now - *ping_sent).checked_sub(Duration::from_micros(queue_time))
                    {
                        stats.samples.push_back(Sample {
                            when,
                            rtt_us: rtt.as_micros().try_into().unwrap_or(std::i64::MAX),
                        });
                    }
                }
                stats.recalculate(state).await?;
            }
            Ok(())
        },
        async move {
            let mut stats_lock = PollMutex::new(stats_ref);
            let mut poll_timeout = |ctx: &mut Context<'_>, current_timeout: Option<Instant>| {
                let mut stats = ready!(stats_lock.poll(ctx));
                if stats.timeout != current_timeout {
                    Poll::Ready(Some(stats.timeout))
                } else if stats.closed {
                    Poll::Ready(None)
                } else {
                    stats.wake_on_change_timeout = Some(ctx.waker().clone());
                    Poll::Pending
                }
            };
            let mut current_timeout = None;
            loop {
                match futures::future::select(
                    poll_fn(|ctx| poll_timeout(ctx, current_timeout)),
                    maybe_wait_until(current_timeout).boxed(),
                )
                .await
                {
                    Either::Left((Some(timeout), _)) => {
                        current_timeout = timeout;
                    }
                    Either::Left((None, _)) => return Ok(()),
                    Either::Right(_) => {
                        let mut state = state_ref.lock().await;
                        let mut stats = stats_ref.lock().await;
                        stats.timeout = None;
                        stats.garbage_collect();
                        state.send_ping = true;
                        state.on_send_ping_pong.take().map(|w| w.wake());
                        stats.recalculate(state).await?;
                    }
                }
            }
        },
    )
    .await?;
    Ok(())
}

/// Schedule pings when they need to be scheduled, provide an estimation of round trip time
pub struct PingTracker(Arc<Mutex<State>>);

fn square(a: i64) -> Option<i64> {
    a.checked_mul(a)
}

impl Drop for PingTracker {
    fn drop(&mut self) {
        let state = self.0.clone();
        Task::spawn(async move {
            let mut state = state.lock().await;
            state.closed = true;
            state.on_closed.take().map(|w| w.wake());
        })
        .detach();
    }
}

impl PingTracker {
    /// Setup a new (empty) PingTracker
    pub fn new() -> PingTracker {
        let state = Arc::new(Mutex::new(State {
            round_trip_time: Observable::new(None),
            send_ping: true,
            send_pong: None,
            on_send_ping_pong: None,
            sent_ping: None,
            on_sent_ping: None,
            received_pong: None,
            on_received_pong: None,
            closed: false,
            on_closed: None,
            next_ping_id: 1,
        }));
        Self(state)
    }

    pub fn run(&self) -> impl Future<Output = Result<(), Error>> {
        ping_pong(self.0.clone())
    }

    /// Query current round trip time
    pub async fn new_round_trip_time_observer(&self) -> Observer<Option<Duration>> {
        self.0.lock().await.round_trip_time.new_observer()
    }

    pub async fn round_trip_time(&self) -> Option<Duration> {
        self.0.lock().await.round_trip_time.current().await
    }

    pub fn poll_send_ping_pong(&self, ctx: &mut Context<'_>) -> (Option<u64>, Option<Pong>) {
        let mut state = match Pin::new(&mut self.0.lock()).poll(ctx) {
            Poll::Pending => return (None, None),
            Poll::Ready(x) => x,
        };
        let ping = if state.send_ping {
            state.send_ping = false;
            let id = state.next_ping_id;
            state.next_ping_id += 1;
            let now = Instant::now();
            state.sent_ping = Some((id, now));
            state.on_sent_ping.take().map(|w| w.wake());
            Some(id)
        } else {
            None
        };
        let pong = if let Some((id, ping_received)) = state.send_pong.take() {
            let queue_time =
                (Instant::now() - ping_received).as_micros().try_into().unwrap_or(std::u64::MAX);
            Some(Pong { id, queue_time })
        } else {
            None
        };
        if ping.is_none() && pong.is_none() {
            state.on_send_ping_pong = Some(ctx.waker().clone());
        }
        (ping, pong)
    }

    /// Upon receiving a pong: return a set of operations that need to be scheduled
    pub async fn got_pong(&self, pong: Pong) {
        let mut state = self.0.lock().await;
        state.received_pong = Some((pong, Instant::now()));
        state.on_received_pong.take().map(|w| w.wake());
    }

    pub async fn got_ping(&self, ping: u64) {
        let mut state = self.0.lock().await;
        state.send_pong = Some((ping, Instant::now()));
        state.on_send_ping_pong.take().map(|w| w.wake());
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::router::test_util::run;
    use crate::runtime::wait_until;
    use futures::task::noop_waker;

    #[test]
    fn published_mean_updates() {
        run(|| async move {
            let pt = PingTracker::new();
            let pt_run = pt.run();
            let _pt_run = Task::spawn(async move {
                pt_run.await.unwrap();
            });
            let mut rtt_obs = pt.new_round_trip_time_observer().await;
            assert_eq!(
                pt.poll_send_ping_pong(&mut Context::from_waker(&noop_waker())),
                (Some(1), None)
            );
            assert_eq!(rtt_obs.next().await, Some(None));
            wait_until(Instant::now() + Duration::from_secs(1)).await;
            pt.got_pong(Pong { id: 1, queue_time: 100 }).await;
            let next = rtt_obs.next().await;
            assert_ne!(next, None);
            assert_ne!(next, Some(None));
        })
    }
}
