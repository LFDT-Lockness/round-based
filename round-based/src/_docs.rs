use futures_util::{Sink, SinkExt, Stream};

use crate::{Incoming, Outgoing};

pub fn fake_delivery<M, E>()
-> impl Stream<Item = Result<Incoming<M>, E>> + Sink<Outgoing<M>, Error = E> + Unpin {
    crate::mpc::Halves::new(
        futures_util::stream::pending::<Result<Incoming<M>, E>>(),
        futures_util::sink::drain().sink_map_err(|e| match e {}),
    )
}
