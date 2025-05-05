use round_based::ProtocolMsg;

#[derive(ProtocolMsg)]
enum Msg {
    // Unnamed variant with single field is the only correct enum variant
    // that doesn't contradicts with ProtocolMsg derivation
    VariantA(u16),
    // Error: You can't have named variants
    VariantB { n: u32 },
    // Error: Variant must have exactly 1 field
    VariantC(u32, String),
    // Error: Variant must have exactly 1 field!!
    VariantD(),
    // Error: Union variants are not permitted
    VariantE,
}

// Structure cannot implement ProtocolMsg
#[derive(ProtocolMsg)]
struct Msg2 {
    some_field: u64,
}

// Union cannot implement ProtocolMsg
#[derive(ProtocolMsg)]
union Msg3 {
    variant: u64,
}

// protocol_message is repeated twice
#[derive(ProtocolMsg)]
#[protocol_message(root = one)]
#[protocol_message(root = two)]
enum Msg4 {
    One(u32),
    Two(u16),
}

// ", blah blah" is not permitted input
#[derive(ProtocolMsg)]
#[protocol_message(root = one, blah blah)]
enum Msg5 {
    One(u32),
    Two(u16),
}

// `protocol_message` must not be empty
#[derive(ProtocolMsg)]
#[protocol_message()]
enum Msg6 {
    One(u32),
    Two(u16),
}

fn main() {}
