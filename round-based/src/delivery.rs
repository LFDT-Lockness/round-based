/// Incoming message
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Incoming<M> {
    /// Index of a message
    pub id: MsgId,
    /// Index of a party who sent the message
    pub sender: PartyIndex,
    /// Indicates whether it's a broadcast message (meaning that this message is received by all the
    /// parties), or p2p (private message sent by `sender`)
    pub msg_type: MessageType,
    /// Received message
    pub msg: M,
}

/// Message type (broadcast or p2p)
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum MessageType {
    /// Message was broadcasted
    Broadcast {
        /// Indicates that message was reliably broadcasted, meaning that it's guaranteed (cryptographically or through
        /// other trust assumptions) that all honest participants of the protocol received the same message
        reliable: bool,
    },
    /// P2P message
    P2P,
}

/// Index of party involved in the protocol
pub type PartyIndex = u16;
/// ID of received message
///
/// Can be used to retrieve extra information about message from delivery layer when needed.
/// E.g. if malicious party is detected, we need a proof that received message was sent by this
/// party, so message id should be used to retrieve signature and original message.
pub type MsgId = u64;

impl<M> Incoming<M> {
    /// Maps `Incoming<M>` to `Incoming<T>` by applying a function to the message body
    pub fn map<T, F>(self, f: F) -> Incoming<T>
    where
        F: FnOnce(M) -> T,
    {
        Incoming {
            id: self.id,
            sender: self.sender,
            msg_type: self.msg_type,
            msg: f(self.msg),
        }
    }

    /// Maps `Incoming<M>` to `Result<Incoming<T>, E>` by applying a function `fn(M) -> Result<T, E>`
    /// to the message body
    pub fn try_map<T, E, F>(self, f: F) -> Result<Incoming<T>, E>
    where
        F: FnOnce(M) -> Result<T, E>,
    {
        Ok(Incoming {
            id: self.id,
            sender: self.sender,
            msg_type: self.msg_type,
            msg: f(self.msg)?,
        })
    }

    /// Converts `&Incoming<M>` to `Incoming<&M>`
    pub fn as_ref(&self) -> Incoming<&M> {
        Incoming {
            id: self.id,
            sender: self.sender,
            msg_type: self.msg_type,
            msg: &self.msg,
        }
    }

    /// Checks whether it's broadcast message (regardless if it's reliable or not)
    pub fn is_broadcast(&self) -> bool {
        matches!(self.msg_type, MessageType::Broadcast { .. })
    }

    /// Checks if message was reliably broadcasted
    pub fn is_reliably_broadcasted(&self) -> bool {
        matches!(self.msg_type, MessageType::Broadcast { reliable: true })
    }

    /// Checks whether it's p2p message
    pub fn is_p2p(&self) -> bool {
        matches!(self.msg_type, MessageType::P2P)
    }
}

/// Outgoing message
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outgoing<M> {
    /// Message destination: either one party (p2p message) or all parties (broadcast message)
    pub recipient: MessageDestination,
    /// Message being sent
    pub msg: M,
}

impl<M> Outgoing<M> {
    /// Constructs an outgoing message addressed to all parties
    pub fn all_parties(msg: M) -> Self {
        Self {
            recipient: MessageDestination::AllParties { reliable: false },
            msg,
        }
    }

    /// Constructs an outgoing message addressed to all parties via reliable broadcast channel
    pub fn reliable_broadcast(msg: M) -> Self {
        Self {
            recipient: MessageDestination::AllParties { reliable: true },
            msg,
        }
    }

    /// Constructs an outgoing message addressed to one party
    pub fn p2p(recipient: PartyIndex, msg: M) -> Self {
        Self {
            recipient: MessageDestination::OneParty(recipient),
            msg,
        }
    }

    /// Maps `Outgoing<M>` to `Outgoing<M2>` by applying a function to the message body
    pub fn map<M2, F>(self, f: F) -> Outgoing<M2>
    where
        F: FnOnce(M) -> M2,
    {
        Outgoing {
            recipient: self.recipient,
            msg: f(self.msg),
        }
    }

    /// Converts `&Outgoing<M>` to `Outgoing<&M>`
    pub fn as_ref(&self) -> Outgoing<&M> {
        Outgoing {
            recipient: self.recipient,
            msg: &self.msg,
        }
    }

    /// Checks whether it's broadcast message
    pub fn is_broadcast(&self) -> bool {
        self.recipient.is_broadcast()
    }

    /// Checks whether it's a reliable broadcast message
    pub fn is_reliable_broadcast(&self) -> bool {
        self.recipient.is_reliable_broadcast()
    }

    /// Checks whether it's p2p message
    pub fn is_p2p(&self) -> bool {
        self.recipient.is_p2p()
    }
}

/// Destination of an outgoing message
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum MessageDestination {
    /// Broadcast message
    AllParties {
        /// Indicates that message needs to be reliably broadcasted, meaning that when recipient receives this message,
        /// it must be assured (cryptographically or through other trust assumptions) that all honest participants of the
        /// protocol received the same message
        reliable: bool,
    },
    /// P2P message
    OneParty(PartyIndex),
}

impl MessageDestination {
    /// Returns `true` if it's p2p message
    pub fn is_p2p(&self) -> bool {
        matches!(self, MessageDestination::OneParty(_))
    }
    /// Returns `true` if it's broadcast message (regardless if it's reliable or not)
    pub fn is_broadcast(&self) -> bool {
        matches!(self, MessageDestination::AllParties { .. })
    }
    /// Returns `true` if it's reliable broadcast message
    pub fn is_reliable_broadcast(&self) -> bool {
        matches!(self, MessageDestination::AllParties { reliable: true })
    }
}
