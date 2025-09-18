//! Primitives that process and collect messages received at certain round

use core::any::Any;

use crate::Incoming;

pub use self::simple_store::{
    broadcast, p2p, reliable_broadcast, RoundInput, RoundInputError, RoundMsgs,
};

mod simple_store;

/// Common information about a round
pub trait RoundInfo: Sized + 'static {
    /// Message type
    type Msg;
    /// Store output (e.g. `Vec<_>` of received messages)
    type Output;
    /// Store error
    type Error: core::error::Error;
}

/// Stores messages received at particular round
///
/// In MPC protocol, party at every round usually needs to receive up to `n` messages. `RoundsStore`
/// is a container that stores messages, it knows how many messages are expected to be received,
/// and should implement extra measures against malicious parties (e.g. prohibit message overwrite).
///
/// ## Flow
/// `RoundStore` stores received messages. Once enough messages are received, it outputs [`RoundInfo::Output`].
/// In order to save received messages, [`.add_message(msg)`] is called. Then, [`.wants_more()`] tells whether more
/// messages are needed to be received. If it returned `false`, then output can be retrieved by calling [`.output()`].
///
/// [`.add_message(msg)`]: Self::add_message
/// [`.wants_more()`]: Self::wants_more
/// [`.output()`]: Self::output
///
/// ## Example
/// [`RoundInput`] is an simple messages store. Refer to its docs to see usage examples.
pub trait RoundStore: RoundInfo {
    /// Adds received message to the store
    ///
    /// Returns error if message cannot be processed. Usually it means that sender behaves maliciously.
    fn add_message(&mut self, msg: Incoming<Self::Msg>) -> Result<(), Self::Error>;
    /// Indicates if store expects more messages to receive
    fn wants_more(&self) -> bool;
    /// Retrieves store output if enough messages are received
    ///
    /// Returns `Err(self)` if more message are needed to be received.
    ///
    /// If store indicated that it needs no more messages (ie `store.wants_more() == false`), then
    /// this function must return `Ok(_)`.
    fn output(self) -> Result<Self::Output, Self>;

    /// Interface that exposes ability to retrieve generic information about the round store
    ///
    /// For reading store properties, it's recommended to use [`RoundStoreExt::read_prop`] method which
    /// uses this function internally.
    ///
    /// When implementing `RoundStore` trait, if you wish to expose no extra information, leave the default
    /// implementation of this method. If you do want to expose certain properties that will be accessible
    /// through [`RoundStoreExt::read_prop`], follow this example:
    ///
    /// ```rust
    /// pub struct MyStore { /* ... */ }
    ///
    /// #[derive(Debug, PartialEq, Eq)]
    /// pub struct SomePropertyWeWantToExpose { value: u64 }
    /// #[derive(Debug, PartialEq, Eq)]
    /// pub struct AnotherProperty(String);
    ///
    /// # type Msg = ();
    /// # impl round_based::round::RoundInfo for MyStore {
    /// #    type Msg = Msg;
    /// #    type Output = Vec<Msg>;
    /// #    type Error = core::convert::Infallible;
    /// # }
    /// impl round_based::round::RoundStore for MyStore {
    /// #    fn add_message(&mut self, msg: round_based::Incoming<Self::Msg>) -> Result<(), Self::Error> { unimplemented!() }
    /// #    fn wants_more(&self) -> bool { unimplemented!() }
    /// #    fn output(self) -> Result<Self::Output, Self> { unimplemented!() }
    ///     // ...
    ///
    ///     fn read_any_prop(&self, property: &mut dyn core::any::Any) {
    ///         if let Some(p) = property.downcast_mut::<Option<SomePropertyWeWantToExpose>>() {
    ///             *p = Some(SomePropertyWeWantToExpose { value: 42 })
    ///         } else if let Some(p) = property.downcast_mut::<Option<AnotherProperty>>() {
    ///             *p = Some(AnotherProperty("here we return a string".to_owned()))
    ///         }
    ///     }
    /// }
    ///
    /// // Which then can be accessed via `.read_prop()` method:
    /// use round_based::round::RoundStoreExt;
    /// let store = MyStore { /* ... */ };
    /// assert_eq!(
    ///     store.read_prop::<SomePropertyWeWantToExpose>(),
    ///     Some(SomePropertyWeWantToExpose { value: 42 }),
    /// );
    /// assert_eq!(
    ///     store.read_prop::<AnotherProperty>(),
    ///     Some(AnotherProperty("here we return a string".to_owned())),
    /// );
    /// ```
    fn read_any_prop(&self, property: &mut dyn Any) {
        let _ = property;
    }
}

/// Extra functionalities defined for any [`RoundStore`]
pub trait RoundStoreExt: RoundStore {
    /// Reads a property `P` of the store
    ///
    /// Returns `Some(property_value)` if this store exposes property `P`, otherwise returns `None`
    fn read_prop<P: Any>(&self) -> Option<P>;

    /// Constructs a new store that exposes property `P` with provided value
    ///
    /// If store already provides a property `P`, it will be overwritten
    fn set_prop<P: Clone + 'static>(self, value: P) -> WithProp<P, Self>;
}

impl<S: RoundStore> RoundStoreExt for S {
    fn read_prop<P: Any>(&self) -> Option<P> {
        let mut p: Option<P> = None;
        self.read_any_prop(&mut p);
        p
    }

    fn set_prop<P: Clone + 'static>(self, value: P) -> WithProp<P, Self> {
        WithProp {
            prop: value,
            store: self,
        }
    }
}

/// Returned by [`RoundStoreExt::set_prop`]
pub struct WithProp<P, S> {
    prop: P,
    store: S,
}

impl<P, S> RoundInfo for WithProp<P, S>
where
    S: RoundInfo,
    P: 'static,
{
    type Msg = S::Msg;
    type Output = S::Output;
    type Error = S::Error;
}

impl<P, S> RoundStore for WithProp<P, S>
where
    S: RoundStore,
    P: Clone + 'static,
{
    #[inline(always)]
    fn add_message(&mut self, msg: Incoming<Self::Msg>) -> Result<(), Self::Error> {
        self.store.add_message(msg)
    }
    #[inline(always)]
    fn wants_more(&self) -> bool {
        self.store.wants_more()
    }
    #[inline(always)]
    fn output(self) -> Result<Self::Output, Self> {
        self.store.output().map_err(|store| Self {
            prop: self.prop,
            store,
        })
    }

    fn read_any_prop(&self, property: &mut dyn Any) {
        if let Some(p) = property.downcast_mut::<Option<P>>() {
            *p = Some(self.prop.clone())
        } else {
            self.store.read_any_prop(property);
        }
    }
}

/// Properties that may be exposed by [`RoundStore`]
pub mod props {
    /// Indicates whether the round requires messages to be reliably broadcasted
    pub struct RequiresReliableBroadcast(pub bool);
}
