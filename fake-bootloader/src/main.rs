#![no_std]
#![no_main]
extern crate alloc;

use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::device_path::text::{DevicePathToText, DisplayOnly, AllowShortcuts};
use core::fmt::Write;

#[entry]
fn efi_main() -> Status {
    uefi::helpers::init().expect("init");

    // Get our own loaded image path
    let loaded_image = uefi::boot::open_protocol_exclusive::<LoadedImage>(
        uefi::boot::image_handle()
    ).expect("LoadedImage");

    let mut name = alloc::string::String::from("unknown");

    // Try to get the file path from loaded image's file_path()
    if let Some(file_path) = loaded_image.file_path() {
        if let Ok(to_text) = uefi::boot::get_handle_for_protocol::<DevicePathToText>()
            .and_then(|h| uefi::boot::open_protocol_exclusive::<DevicePathToText>(h))
        {
            if let Ok(text) = to_text.convert_device_path_to_text(
                file_path,
                DisplayOnly(true),
                AllowShortcuts(false),
            ) {
                name = alloc::format!("{}", &*text);
            }
        }
    }

    // Print sentinel to serial/console
    let msg = alloc::format!("BOOTED: {}\n", name);
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.write_str(&msg);
    });

    // Stall to allow the serial port buffer to flush
    uefi::boot::stall(core::time::Duration::from_millis(500));

    // Shutdown the VM
    uefi::runtime::reset(uefi::runtime::ResetType::SHUTDOWN, Status::SUCCESS, None);
}
