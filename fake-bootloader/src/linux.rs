#![no_std]
#![no_main]

use uefi::prelude::*;
use core::fmt::Write;

#[entry]
fn efi_main() -> Status {
    uefi::helpers::init().expect("init");

    uefi::system::with_stdout(|stdout| {
        let _ = stdout.write_str("BOOTED: \\vmlinuz-test\n");
    });

    // Stall for 500ms to allow serial TX buffer to flush
    uefi::boot::stall(core::time::Duration::from_millis(500));

    // Shutdown VM
    uefi::runtime::reset(uefi::runtime::ResetType::SHUTDOWN, Status::SUCCESS, None);
}
