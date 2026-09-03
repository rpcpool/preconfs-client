//! Typed event streams over the raw gRPC streams, with reconnect.
//!
//! [`EventStream`] turns the proto updates into [`Event`]s, drops pings,
//! and when the connection fails it resubscribes according to the
//! [`Reconnect`] schedule set on the [`Connector`](crate::Connector). It is
//! a `Stream` decorator: the in-flight resubscribe is a future it polls
//! from `poll_next`, no task and no channel in between.

use {
    crate::{
        connect::Client,
        error::StreamError,
        reconnect::{Reconnect, retryable},
    },
    futures_core::Stream,
    std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll, ready},
    },
    tonic::{Status, Streaming},
    triton_preconfs_proto::preconfs::{
        BamTransaction, BamUpdate, HarmonicTransaction, HarmonicUpdate, SubscribeRequest,
        bam_update, harmonic_update,
    },
};

/// A transaction the server delivered, with the names of the filters that
/// matched it.
#[derive(Debug, Clone, PartialEq)]
pub struct Matched<T> {
    /// Names from the subscribe request, at least one.
    pub filters: Vec<String>,
    /// The transaction as the feed published it.
    pub transaction: T,
}

/// One item of a stream. `T` is the feed's transaction message.
///
/// Harmonic streams frame each slot: `SlotStart`, its transactions,
/// `SlotEnd`. After `SlotEnd` for a slot the program holds everything its
/// filters matched for it. BAM has no framing; only `Transaction`, `Clip`
/// and `Reconnected` occur, and each transaction names its slot.
#[derive(Debug, Clone, PartialEq)]
pub enum Event<T> {
    /// A leader began streaming preconfs for this slot.
    SlotStart {
        /// The slot.
        slot: u64,
    },
    /// A matching transaction.
    Transaction(Matched<T>),
    /// No further transactions for this slot will arrive.
    SlotEnd {
        /// The slot.
        slot: u64,
    },
    /// Matching transactions were withheld since the previous notice
    /// because the account exceeded its coverage limit. Never silent: the
    /// notice always arrives before the affected slot's `SlotEnd`.
    Clip {
        /// Transactions withheld on this stream since the previous notice.
        transactions: u64,
    },
    /// The stream dropped and was resubscribed. Everything the feed
    /// published in between is gone; on a Harmonic stream framing restarts
    /// at the next `SlotStart`.
    Reconnected {
        /// Failed attempts before this one succeeded.
        attempts: u32,
    },
}

/// One of the two feeds' update messages: how to subscribe to it and how
/// its payload maps to [`Event`]. Implemented for [`HarmonicUpdate`] and
/// [`BamUpdate`].
pub trait FeedUpdate: Sized + Send + 'static {
    /// The feed's transaction message.
    type Transaction: Send;
    /// Whether the feed frames slots. A framed stream that (re)subscribes
    /// while a slot is open holds everything back until the next
    /// `SlotStart`, so a partially observed slot is never mistaken for a
    /// complete one.
    const SLOT_FRAMING: bool;
    /// Opens the raw stream for `request`.
    fn subscribe(
        client: Client,
        request: SubscribeRequest,
    ) -> impl Future<Output = Result<Streaming<Self>, Status>> + Send;
    /// The event for this update; `None` for pings and empty payloads.
    fn into_event(self) -> Option<Event<Self::Transaction>>;
}

impl FeedUpdate for HarmonicUpdate {
    type Transaction = HarmonicTransaction;
    const SLOT_FRAMING: bool = true;

    async fn subscribe(
        client: Client,
        request: SubscribeRequest,
    ) -> Result<Streaming<Self>, Status> {
        Ok(client.harmonic().subscribe(request).await?.into_inner())
    }

    fn into_event(self) -> Option<Event<HarmonicTransaction>> {
        Some(match self.payload? {
            harmonic_update::Payload::Transaction(transaction) => Event::Transaction(Matched {
                filters: self.filters,
                transaction,
            }),
            harmonic_update::Payload::SlotStart(start) => Event::SlotStart { slot: start.slot },
            harmonic_update::Payload::SlotEnd(end) => Event::SlotEnd { slot: end.slot },
            harmonic_update::Payload::Clip(clip) => Event::Clip {
                transactions: clip.transactions,
            },
            harmonic_update::Payload::Ping(_) => return None,
        })
    }
}

impl FeedUpdate for BamUpdate {
    type Transaction = BamTransaction;
    const SLOT_FRAMING: bool = false;

    async fn subscribe(
        client: Client,
        request: SubscribeRequest,
    ) -> Result<Streaming<Self>, Status> {
        Ok(client.bam().subscribe(request).await?.into_inner())
    }

    fn into_event(self) -> Option<Event<BamTransaction>> {
        Some(match self.payload? {
            bam_update::Payload::Transaction(transaction) => Event::Transaction(Matched {
                filters: self.filters,
                transaction,
            }),
            bam_update::Payload::Clip(clip) => Event::Clip {
                transactions: clip.transactions,
            },
            bam_update::Payload::Ping(_) => return None,
        })
    }
}

/// Events of a Harmonic subscription.
pub type HarmonicStream = EventStream<HarmonicUpdate>;
/// Events of a BAM subscription.
pub type BamStream = EventStream<BamUpdate>;
/// Harmonic event.
pub type HarmonicEvent = Event<HarmonicTransaction>;
/// BAM event.
pub type BamEvent = Event<BamTransaction>;

type Connecting<U> = Pin<Box<dyn Future<Output = Result<Streaming<U>, Status>> + Send>>;

// One per stream, not per message: the size gap to `Ended` does not matter
// and boxing the live stream would add an indirection to every poll.
#[allow(clippy::large_enum_variant)]
enum State<U> {
    Streaming(Streaming<U>),
    Waiting(Pin<Box<tokio::time::Sleep>>),
    Connecting(Connecting<U>),
    Ended,
}

/// A subscription as a [`Stream`] of [`Event`]s. Obtained from
/// [`Client::subscribe_harmonic`] and [`Client::subscribe_bam`].
///
/// The stream ends with `None` after yielding the error that ended it, or
/// runs until dropped when reconnect is on and every failure is one that
/// resubscribing fixes.
pub struct EventStream<U: FeedUpdate> {
    client: Client,
    request: SubscribeRequest,
    reconnect: Option<Reconnect>,
    state: State<U>,
    /// Consecutive failed attempts; reset by the first event of a stream.
    attempts: u32,
    /// Whether the current stream has passed its first `SlotStart`; always
    /// true on feeds without framing.
    framed: bool,
}

impl<U: FeedUpdate> EventStream<U> {
    pub(crate) const fn new(
        client: Client,
        request: SubscribeRequest,
        reconnect: Option<Reconnect>,
        stream: Streaming<U>,
    ) -> Self {
        Self {
            client,
            request,
            reconnect,
            state: State::Streaming(stream),
            attempts: 0,
            framed: !U::SLOT_FRAMING,
        }
    }

    /// The next event, `None` once the stream has ended.
    pub async fn next(&mut self) -> Option<Result<Event<U::Transaction>, StreamError>> {
        std::future::poll_fn(|cx| Pin::new(&mut *self).poll_next(cx)).await
    }

    /// Decides what a failure means: schedule a retry, or the error to
    /// yield before ending.
    fn fail(&mut self, error: StreamError) -> Option<StreamError> {
        let Some(reconnect) = &self.reconnect else {
            self.state = State::Ended;
            return Some(error);
        };
        self.attempts += 1;
        if !retryable(&error) || reconnect.exhausted(self.attempts) {
            self.state = State::Ended;
            return Some(error);
        }
        let delay = reconnect.interval(self.attempts);
        self.state = State::Waiting(Box::pin(tokio::time::sleep(delay)));
        None
    }

    fn resubscribe(&mut self) {
        let client = self.client.clone();
        let request = self.request.clone();
        self.state = State::Connecting(Box::pin(U::subscribe(client, request)));
    }
}

impl<U: FeedUpdate> Stream for EventStream<U> {
    type Item = Result<Event<U::Transaction>, StreamError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match &mut this.state {
                State::Ended => return Poll::Ready(None),
                State::Waiting(sleep) => {
                    ready!(sleep.as_mut().poll(cx));
                    this.resubscribe();
                }
                State::Connecting(future) => match ready!(future.as_mut().poll(cx)) {
                    Ok(stream) => {
                        let attempts = this.attempts;
                        this.state = State::Streaming(stream);
                        this.framed = !U::SLOT_FRAMING;
                        return Poll::Ready(Some(Ok(Event::Reconnected { attempts })));
                    }
                    Err(status) => {
                        if let Some(error) = this.fail(StreamError::Status(status)) {
                            return Poll::Ready(Some(Err(error)));
                        }
                    }
                },
                State::Streaming(stream) => {
                    let error = match ready!(Pin::new(stream).poll_next(cx)) {
                        Some(Ok(update)) => {
                            let Some(event) = update.into_event() else {
                                continue;
                            };
                            if !this.framed {
                                if !matches!(event, Event::SlotStart { .. }) {
                                    continue;
                                }
                                this.framed = true;
                            }
                            this.attempts = 0;
                            return Poll::Ready(Some(Ok(event)));
                        }
                        Some(Err(status)) => StreamError::Status(status),
                        None => StreamError::Closed,
                    };
                    if let Some(error) = this.fail(error) {
                        return Poll::Ready(Some(Err(error)));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        triton_preconfs_proto::preconfs::{CoverageClip, HarmonicSlotEnd, HarmonicSlotStart, Ping},
    };

    fn harmonic(payload: harmonic_update::Payload) -> HarmonicUpdate {
        HarmonicUpdate {
            filters: vec!["mine".into()],
            payload: Some(payload),
        }
    }

    #[test]
    fn harmonic_payloads_map_to_events_and_pings_vanish() {
        let start = harmonic(harmonic_update::Payload::SlotStart(HarmonicSlotStart {
            slot: 7,
            region: "ams".into(),
        }));
        assert_eq!(start.into_event(), Some(Event::SlotStart { slot: 7 }));
        let end = harmonic(harmonic_update::Payload::SlotEnd(HarmonicSlotEnd {
            slot: 7,
            region: "ams".into(),
        }));
        assert_eq!(end.into_event(), Some(Event::SlotEnd { slot: 7 }));
        let clip = harmonic(harmonic_update::Payload::Clip(CoverageClip {
            transactions: 3,
        }));
        assert_eq!(clip.into_event(), Some(Event::Clip { transactions: 3 }));
        assert_eq!(
            harmonic(harmonic_update::Payload::Ping(Ping {})).into_event(),
            None
        );
        assert_eq!(
            HarmonicUpdate {
                filters: vec![],
                payload: None
            }
            .into_event(),
            None
        );
        let transaction = harmonic(harmonic_update::Payload::Transaction(HarmonicTransaction {
            slot: 7,
            ..Default::default()
        }));
        match transaction.into_event() {
            Some(Event::Transaction(matched)) => {
                assert_eq!(matched.filters, ["mine"]);
                assert_eq!(matched.transaction.slot, 7);
            }
            other => panic!("expected a transaction, got {other:?}"),
        }
    }

    #[test]
    fn bam_payloads_map_to_events() {
        let update = BamUpdate {
            filters: vec!["mine".into()],
            payload: Some(bam_update::Payload::Transaction(BamTransaction {
                slot: 9,
                ..Default::default()
            })),
        };
        match update.into_event() {
            Some(Event::Transaction(matched)) => assert_eq!(matched.transaction.slot, 9),
            other => panic!("expected a transaction, got {other:?}"),
        }
        assert_eq!(
            BamUpdate {
                filters: vec![],
                payload: Some(bam_update::Payload::Ping(Ping {}))
            }
            .into_event(),
            None
        );
    }
}
