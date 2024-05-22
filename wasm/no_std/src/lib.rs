#![no_std]

use core::panic::PanicInfo;

pub use random_generation_protocol::*;
pub use round_based;

#[panic_handler]
fn panic(_panic: &PanicInfo<'_>) -> ! {
    loop {}
}
