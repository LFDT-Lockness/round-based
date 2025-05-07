use round_based::ProtocolMsg;

#[derive(ProtocolMsg)]
enum Msg<G> {
    VariantA(u16),
    VariantB(String),
    VariantC((u16, String)),
    VariantD(MyStruct<G>),
}
#[derive(ProtocolMsg)]
#[protocol_msg(root = round_based)]
enum Msg2<G> {
    VariantA(u16),
    VariantB(String),
    VariantC((u16, String)),
    VariantD(MyStruct<G>),
}

struct MyStruct<T>(T);

fn main() {}
