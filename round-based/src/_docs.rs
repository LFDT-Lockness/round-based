use core::convert::Infallible;

use phantom_type::PhantomType;

use crate::{Delivery, Incoming, Outgoing};

pub fn fake_delivery<M>() -> impl Delivery<M> {
    struct FakeDelivery<M>(PhantomType<M>);
    impl<M> Delivery<M> for FakeDelivery<M> {
        type Send = futures_util::sink::Drain<Outgoing<M>>;
        type Receive = futures_util::stream::Pending<Result<Incoming<M>, Infallible>>;

        type SendError = Infallible;
        type ReceiveError = Infallible;

        fn split(self) -> (Self::Receive, Self::Send) {
            (futures_util::stream::pending(), futures_util::sink::drain())
        }
    }
    FakeDelivery(PhantomType::new())
}
