//! Simple implementation of `MessagesStore`

use alloc::{vec, vec::Vec};
use core::iter;

use crate::{Incoming, MessageType, MsgId, PartyIndex};

use super::MessagesStore;

/// Round of MPC protocol that waits `n` contributing parties to send a message
///
/// Round takes index of local party `i` and number of contributing parties `n` as parameters,
/// and can be constructed via [`RoundInput::new`] or its aliases: [`RoundInput::p2p`] and
/// [`RoundInput::broadcast`].
///
/// Round is considered complete when the store received a message from every contributing party
/// `j` in `0..n` (except from party `i`, if it's in range). Receiving any message from non-contributing party
/// `n..` is hard error and aborts the round.
///
/// Note that the store will ignore all messages that come from local party `i`.
///
/// Once round is complete, it outputs [`RoundMsgs`].
///
/// ## Example
/// ```rust
/// # use round_based::rounds_router::{MessagesStore, simple_store::RoundInput};
/// # use round_based::{Incoming, MessageType};
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut input = RoundInput::<&'static str>::broadcast(1, 3);
/// input.add_message(Incoming{
///     id: 0,
///     sender: 0,
///     msg_type: MessageType::Broadcast,
///     msg: "first party message",
/// })?;
/// input.add_message(Incoming{
///     id: 1,
///     sender: 2,
///     msg_type: MessageType::Broadcast,
///     msg: "third party message",
/// })?;
/// assert!(!input.wants_more());
///
/// let output = input.output().unwrap();
/// assert_eq!(output.clone().into_vec_without_me(), ["first party message", "third party message"]);
/// assert_eq!(
///     output.clone().into_vec_including_me("my msg"),
///     ["first party message", "my msg", "third party message"]
/// );
/// # Ok(()) }
/// ```
#[derive(Debug, Clone)]
pub struct RoundInput<M> {
    i: PartyIndex,
    n: u16,
    messages_ids: Vec<MsgId>,
    messages: Vec<Option<M>>,
    left_messages: u16,
    expected_msg_type: MessageType,
}

/// List of received messages
#[derive(Debug, Clone)]
pub struct RoundMsgs<M> {
    i: PartyIndex,
    ids: Vec<MsgId>,
    messages: Vec<M>,
}

impl<M> RoundInput<M> {
    /// Constructs new messages store
    ///
    /// Takes index of local party `i` and amount of contributing parties `n` that send messages in
    /// this round. Local party is considered as "non-contributing" if `i >= n`, meaning that it is
    /// not supposed to send messages in this round.
    ///
    /// If number of contributing parties is set to zero `n=0`, the round will accept no messages
    /// and will be finished from the construction: `wants_more()` returns `false`, and `output()`
    /// returns `Ok(received_msgs)` with empty list of received messages.
    pub fn new(i: PartyIndex, n: u16, msg_type: MessageType) -> Self {
        let mut left_messages = n;
        if i < n {
            left_messages -= 1;
        }
        Self {
            i,
            n,
            messages_ids: vec![0; usize::from(left_messages)],
            messages: iter::repeat_with(|| None)
                .take(usize::from(left_messages))
                .collect(),
            left_messages,
            expected_msg_type: msg_type,
        }
    }

    /// Construct a new store for broadcast messages
    ///
    /// The same as [`RoundInput::new(i, n, MessageType::Broadcast)`](RoundInput::new)
    pub fn broadcast(i: PartyIndex, n: u16) -> Self {
        Self::new(i, n, MessageType::Broadcast)
    }

    /// Construct a new store for p2p messages
    ///
    /// The same as [`RoundInput::new(i, n, MessageType::P2P)`](RoundInput::new)
    pub fn p2p(i: PartyIndex, n: u16) -> Self {
        Self::new(i, n, MessageType::P2P)
    }

    fn is_expected_type_of_msg(&self, msg_type: MessageType) -> bool {
        self.expected_msg_type == msg_type
    }
}

impl<M> MessagesStore for RoundInput<M>
where
    M: 'static,
{
    type Msg = M;
    type Output = RoundMsgs<M>;
    type Error = RoundInputError;

    fn add_message(&mut self, msg: Incoming<Self::Msg>) -> Result<(), Self::Error> {
        if !self.is_expected_type_of_msg(msg.msg_type) {
            return Err(RoundInputError::MismatchedMessageType {
                msg_id: msg.id,
                expected: self.expected_msg_type,
                actual: msg.msg_type,
            });
        }
        if msg.sender >= self.n {
            return Err(RoundInputError::SenderIndexOutOfRange {
                msg_id: msg.id,
                sender: msg.sender,
                n: self.n,
            });
        }
        if msg.sender == self.i {
            // Ignore own messages
            return Ok(());
        }

        let index = usize::from(if msg.sender < self.i {
            msg.sender
        } else {
            msg.sender - 1
        });

        match self.messages.get_mut(index) {
            Some(vacant @ None) => {
                *vacant = Some(msg.msg);
                self.messages_ids[index] = msg.id;
                self.left_messages -= 1;
                Ok(())
            }
            Some(Some(_)) => Err(RoundInputError::AttemptToOverwriteReceivedMsg {
                msgs_ids: [self.messages_ids[index], msg.id],
                sender: msg.sender,
            }),
            None => Err(RoundInputError::SenderIndexOutOfRange {
                msg_id: msg.id,
                sender: msg.sender,
                n: self.n,
            }),
        }
    }

    fn wants_more(&self) -> bool {
        self.left_messages > 0
    }

    fn output(self) -> Result<Self::Output, Self> {
        if self.left_messages > 0 {
            Err(self)
        } else {
            Ok(RoundMsgs {
                i: self.i,
                ids: self.messages_ids,
                messages: self.messages.into_iter().flatten().collect(),
            })
        }
    }
}

impl<M> RoundMsgs<M> {
    /// Returns list of received messages
    ///
    /// Messages appear in the list in ascending order of sender index. For instance, for number of
    /// contributing parties `n=4` and local party index `i=2`, the list would look like: `[{msg from
    /// party 0}, {msg from party 1}, {msg from party 3}]`.
    pub fn into_vec_without_me(self) -> Vec<M> {
        self.messages
    }

    /// Returns vec of received messages plus party's own message
    ///
    /// Similar to `into_vec_without_me`, but inserts `my_msg` into output vector.
    ///
    /// Behavior is different depending on `i` and `n` provided at the round construction in
    /// [`RoundInput::new()`]:
    /// * `i < n`, then `my_msg` is inserted at position `i` in resulting list. Thus, i-th message in the
    ///   list was received from i-th party.
    /// * Otherwise, `my_msg` is appended to the list. Thus, i-th message in the list was received
    ///   from i-th party, except for the last message in the list, which corresponds to our own
    ///   message.
    pub fn into_vec_including_me(mut self, my_msg: M) -> Vec<M> {
        if usize::from(self.i) < self.messages.len() {
            self.messages.insert(usize::from(self.i), my_msg);
        } else {
            self.messages.push(my_msg);
        }
        self.messages
    }

    /// Returns iterator over messages
    pub fn iter(&self) -> impl Iterator<Item = &M> {
        self.messages.iter()
    }

    /// Returns iterator over received messages plus party's own message
    ///
    /// Order of the messages is the same as in [`RoundMsgs::into_vec_including_me`]
    pub fn iter_including_me<'m>(&'m self, my_msg: &'m M) -> impl Iterator<Item = &'m M> {
        self.messages
            .iter()
            .take(usize::from(self.i))
            .chain(iter::once(my_msg))
            .chain(self.messages.iter().skip(usize::from(self.i)))
    }

    /// Returns iterator over received messages plus party's own message
    ///
    /// Order of the messages is the same as in [`RoundMsgs::into_vec_including_me`]
    pub fn into_iter_including_me(self, my_msg: M) -> impl Iterator<Item = M> {
        struct InsertsAfter<T, It> {
            offset: usize,
            inner: It,
            item: Option<T>,
        }
        impl<T, It: Iterator<Item = T>> Iterator for InsertsAfter<T, It> {
            type Item = T;
            fn next(&mut self) -> Option<Self::Item> {
                if self.offset == 0 {
                    match self.item.take() {
                        Some(x) => Some(x),
                        None => self.inner.next(),
                    }
                } else {
                    self.offset -= 1;
                    self.inner.next()
                }
            }
        }
        InsertsAfter {
            offset: usize::from(self.i).min(self.messages.len()),
            inner: self.messages.into_iter(),
            item: Some(my_msg),
        }
    }

    /// Returns iterator over messages with sender indexes
    ///
    /// Iterator yields `(sender_index, msg_id, message)`
    pub fn into_iter_indexed(self) -> impl Iterator<Item = (PartyIndex, MsgId, M)> {
        let parties_indexes = (0..self.i).chain(self.i + 1..);
        parties_indexes
            .zip(self.ids)
            .zip(self.messages)
            .map(|((party_ind, msg_id), msg)| (party_ind, msg_id, msg))
    }

    /// Returns iterator over messages with sender indexes
    ///
    /// Iterator yields `(sender_index, msg_id, &message)`
    pub fn iter_indexed(&self) -> impl Iterator<Item = (PartyIndex, MsgId, &M)> {
        let parties_indexes = (0..self.i).chain(self.i + 1..);
        parties_indexes
            .zip(&self.ids)
            .zip(&self.messages)
            .map(|((party_ind, msg_id), msg)| (party_ind, *msg_id, msg))
    }
}

/// Error explaining why `RoundInput` wasn't able to process a message
#[derive(Debug, thiserror::Error)]
pub enum RoundInputError {
    /// Party sent two messages in one round
    ///
    /// `msgs_ids` are ids of conflicting messages
    #[error("party {sender} tried to overwrite message")]
    AttemptToOverwriteReceivedMsg {
        /// IDs of conflicting messages
        msgs_ids: [MsgId; 2],
        /// Index of party who sent two messages in one round
        sender: PartyIndex,
    },
    /// Unknown sender
    ///
    /// This error is thrown when index of sender is not in `[0; n)` where `n` is number of
    /// parties involved in the protocol (provided in [`RoundInput::new`])
    #[error("sender index is out of range: sender={sender}, n={n}")]
    SenderIndexOutOfRange {
        /// Message ID
        msg_id: MsgId,
        /// Sender index
        sender: PartyIndex,
        /// Number of parties
        n: u16,
    },
    /// Received message type doesn't match expectations
    ///
    /// For instance, this error is returned when it's expected to receive broadcast message,
    /// but party sent p2p message instead (which is rough protocol violation).
    #[error("expected message {expected:?}, got {actual:?}")]
    MismatchedMessageType {
        /// Message ID
        msg_id: MsgId,
        /// Expected type of message
        expected: MessageType,
        /// Actual type of message
        actual: MessageType,
    },
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use matches::assert_matches;

    use crate::rounds_router::store::MessagesStore;
    use crate::{Incoming, MessageType};

    use super::{RoundInput, RoundInputError};

    #[derive(Debug, Clone, PartialEq)]
    pub struct Msg(u16);

    #[test]
    fn store_outputs_received_messages() {
        for i in 0..10 {
            let mut store = RoundInput::<Msg>::new(i, 5, MessageType::P2P);

            let msgs = (0..5)
                .map(|s| Incoming {
                    id: s.into(),
                    sender: s,
                    msg_type: MessageType::P2P,
                    msg: Msg(10 + s),
                })
                .filter(|incoming| incoming.sender != i)
                .collect::<Vec<_>>();

            for msg in &msgs {
                assert!(store.wants_more());
                store.add_message(msg.clone()).unwrap();
            }

            assert!(!store.wants_more());
            let received = store.output().unwrap();

            // without me
            let msgs: Vec<_> = msgs.into_iter().map(|msg| msg.msg).collect();
            assert_eq!(received.clone().into_vec_without_me(), msgs);

            // including me
            let my_msg = Msg(10 + i);
            let received = received.into_vec_including_me(my_msg.clone());
            if i < 5 {
                let i = usize::from(i);
                assert_eq!(received[0..i], msgs[0..i]);
                assert_eq!(received[i], my_msg);
                assert_eq!(received[i + 1..5], msgs[i..4]);
            } else {
                assert_eq!(received[..5], msgs);
                assert_eq!(received[5], my_msg)
            }
        }
    }

    #[test]
    fn store_returns_error_if_sender_index_is_out_of_range() {
        for i in 0..10 {
            std::dbg!(i);
            let mut store = RoundInput::new(i, 5, MessageType::P2P);
            let error = store
                .add_message(Incoming {
                    id: 0,
                    sender: 5,
                    msg_type: MessageType::P2P,
                    msg: Msg(123),
                })
                .unwrap_err();
            assert_matches!(
                error,
                RoundInputError::SenderIndexOutOfRange { msg_id, sender, n } if msg_id == 0 && sender == 5 && n == 5
            );
        }
    }

    #[test]
    fn store_returns_error_if_incoming_msg_overwrites_already_received_one() {
        for i in [0, 5] {
            let mut store = RoundInput::new(i, 3, MessageType::P2P);
            store
                .add_message(Incoming {
                    id: 0,
                    sender: 1,
                    msg_type: MessageType::P2P,
                    msg: Msg(11),
                })
                .unwrap();
            let error = store
                .add_message(Incoming {
                    id: 1,
                    sender: 1,
                    msg_type: MessageType::P2P,
                    msg: Msg(112),
                })
                .unwrap_err();
            assert_matches!(error, RoundInputError::AttemptToOverwriteReceivedMsg { msgs_ids, sender } if msgs_ids[0] == 0 && msgs_ids[1] == 1 && sender == 1);
            store
                .add_message(Incoming {
                    id: 2,
                    sender: 2,
                    msg_type: MessageType::P2P,
                    msg: Msg(22),
                })
                .unwrap();
            if i != 0 {
                store
                    .add_message(Incoming {
                        id: 3,
                        sender: 0,
                        msg_type: MessageType::P2P,
                        msg: Msg(33),
                    })
                    .unwrap();
            }

            let output = store.output().unwrap().into_vec_without_me();
            if i == 0 {
                assert_eq!(output, [Msg(11), Msg(22)]);
            } else {
                assert_eq!(output, [Msg(33), Msg(11), Msg(22)]);
            }
        }
    }

    #[test]
    fn store_returns_error_if_tried_to_output_before_receiving_enough_messages() {
        for i in [3, 10] {
            let mut store = RoundInput::<Msg>::new(i, 5, MessageType::P2P);

            let msgs = (0..5)
                .map(|s| Incoming {
                    id: s.into(),
                    sender: s,
                    msg_type: MessageType::P2P,
                    msg: Msg(10 + s),
                })
                .filter(|incoming| incoming.sender != i);

            for msg in msgs {
                assert!(store.wants_more());
                store = store.output().unwrap_err();

                store.add_message(msg).unwrap();
            }

            let _ = store.output().unwrap();
        }
    }

    #[test]
    fn store_returns_error_if_message_type_mismatched() {
        let mut store = RoundInput::<Msg>::p2p(3, 5);
        let err = store
            .add_message(Incoming {
                id: 0,
                sender: 0,
                msg_type: MessageType::Broadcast,
                msg: Msg(1),
            })
            .unwrap_err();
        assert_matches!(
            err,
            RoundInputError::MismatchedMessageType {
                msg_id: 0,
                expected: MessageType::P2P,
                actual: MessageType::Broadcast
            }
        );

        let mut store = RoundInput::<Msg>::broadcast(3, 5);
        let err = store
            .add_message(Incoming {
                id: 0,
                sender: 0,
                msg_type: MessageType::P2P,
                msg: Msg(1),
            })
            .unwrap_err();
        assert_matches!(
            err,
            RoundInputError::MismatchedMessageType {
                msg_id: 0,
                expected: MessageType::Broadcast,
                actual: MessageType::P2P,
            }
        );
        for sender in 0u16..5 {
            store
                .add_message(Incoming {
                    id: 0,
                    sender,
                    msg_type: MessageType::Broadcast,
                    msg: Msg(1),
                })
                .unwrap();
        }

        let mut store = RoundInput::<Msg>::broadcast(3, 5);
        let err = store
            .add_message(Incoming {
                id: 0,
                sender: 0,
                msg_type: MessageType::P2P,
                msg: Msg(1),
            })
            .unwrap_err();
        assert_matches!(
            err,
            RoundInputError::MismatchedMessageType {
                msg_id: 0,
                expected: MessageType::Broadcast,
                actual,
            } if actual == MessageType::P2P
        );
        store
            .add_message(Incoming {
                id: 0,
                sender: 0,
                msg_type: MessageType::Broadcast,
                msg: Msg(1),
            })
            .unwrap();
    }

    #[test]
    fn into_iter_including_me() {
        let me = -10_isize;
        let messages = alloc::vec![1, 2, 3];

        let me_first = super::RoundMsgs {
            i: 0,
            ids: alloc::vec![1, 2, 3],
            messages: messages.clone(),
        };
        let all = me_first.into_iter_including_me(me).collect::<Vec<_>>();
        assert_eq!(all, [-10, 1, 2, 3]);

        let me_second = super::RoundMsgs {
            i: 1,
            ids: alloc::vec![0, 2, 3],
            messages: messages.clone(),
        };
        let all = me_second.into_iter_including_me(me).collect::<Vec<_>>();
        assert_eq!(all, [1, -10, 2, 3]);

        for i in [3, 10] {
            let me_last = super::RoundMsgs {
                i,
                ids: alloc::vec![0, 1, 2],
                messages: messages.clone(),
            };
            let all = me_last.into_iter_including_me(me).collect::<Vec<_>>();
            assert_eq!(all, [1, 2, 3, -10]);
        }
    }

    #[test]
    fn round_with_no_contributing_parties() {
        let round = RoundInput::<i32>::p2p(3, 0);

        // Ready from the construction
        assert!(!round.wants_more());

        // Returns empty list of received messages
        let msgs = round.output().unwrap();
        assert_eq!(msgs.iter().count(), 0);
        let expected: &[i32] = &[];
        assert_eq!(msgs.into_vec_without_me(), expected);

        // Accepts no messages
        let mut round = RoundInput::<i32>::p2p(3, 0);
        for sender in 0..=5 {
            round
                .add_message(Incoming {
                    id: 0,
                    sender,
                    msg_type: MessageType::P2P,
                    msg: 67,
                })
                .unwrap_err();
        }
    }
}
