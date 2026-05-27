# UEFI Bootloader with Slint GUI & HTTP Web Server

A premium, custom UEFI bootloader written in Rust featuring a beautiful local graphical interface (built with the Slint UI framework) and a background TCP/IP networking stack (built with smoltcp) serving an HTTP web console. It enables you to select and boot your operating systems (Windows or Linux) either locally with a keyboard/mouse or remotely over your local network.

---

## Key Features

* **Direct Booting (Windows & Linux)**:
  * **Windows**: Directly executes Windows Boot Manager (`\EFI\Microsoft\Boot\bootmgfw.efi`), resolving path reference boundaries by properly setting `DevicePath` metadata in the `LoadedImage` protocol.
  * **Linux (EFI Stub)**: Parses systemd-boot configuration files under `\loader\entries\*.conf`. It automatically extracts the kernel image, `initrd` files, and command-line parameters (`options`), links them into a unified argument buffer, sets load options on the kernel handle, and executes the Linux EFI stub directly, bypassing systemd-boot timeout delays.
* **Real-Time On-Screen Log Terminal**:
  * An integrated green console terminal at the bottom of the screen displays real-time execution logs (network status, DHCP lease, loading status).
  * Automatically handles scrolling to the latest log line as logs are output, while preserving manual mouse wheel scrolling to check history.
  * Duplicates all log messages to the standard UEFI stdout (console/serial port) for easy kernel/QEMU debugging.
* **Premium Graphical Performance & Memory Safety**:
  * Automatically sets the best physical screen mode up to **1080p (1920x1080)**. This caps memory consumption to a safe **8.2MB** frame buffer, preventing UEFI heap pool exhaustion (OOM) and immediate motherboards power-offs associated with native 4K software buffers (which require contiguous 33MB allocations).
  * Implements dynamic DPI scaling (logical scale factor) to keep the text, buttons, and layouts crisp and perfectly sized.
* **Snappy Mouse Control**:
  * High-performance cursor tracking using raw mouse coordinate counts, calibrated and scaled for consistent responsiveness.
  * Supports scroll wheel scrolling (Z-axis pointer events) mapped directly to Slint's scroll viewport.
* **Responsive Mobile Web Console**:
  * Serves a responsive web application from the embedded HTTP daemon.
  * The interface scales dynamically to fit mobile devices, stacking buttons vertically and resizing text for easy operation on smartphones.
* **Clean Handover & Re-entrancy Guards**:
  * Restores the standard UEFI text console mode (`stdout.reset(false)`) right before starting the OS kernel to ensure early kernel/boot logs draw visibly.
  * Includes an execution guard to prevent recursive re-entrant logger calls from triggering stacks overflows during render updates.

---

## File Structure

* `src/main.rs`: Entry point. Initializes the network stack, custom logger, Slint platform, and drives the event loop.
* `src/slint_plat.rs`: Custom Slint platform implementation. Handles graphics mode initialization, mouse cursor rendering, raw pointer tracking, and force-flushing logs.
* `src/boot.rs`: Parses systemd-boot configs, configures loaded image option buffers, manages display mode resets, and handles handover to target EFI images.
* `src/net.rs`: Direct UEFI Simple Network Protocol (SNP) driver wrapper for smoltcp.
* `src/web.rs`: Embedded HTTP server endpoint handler.
* `ui/appwindow.slint`: Centered local UI definition, styling, buttons, and log console scrolling layout.
* `Makefile`: Standard shortcuts to compile targeting UEFI and mount the binaries into the ESP folder.

---

## Building and Running

### Prerequisites

You need a Rust toolchain with the target `x86_64-unknown-uefi` installed.

```bash
rustup target add x86_64-unknown-uefi
```

### Compile

Compile the project and stage it in the EFI system partition (`esp`) folder:

```bash
make
```

The output bootable binary will be copied to `esp/EFI/BOOT/BOOTX64.EFI`, ready to be loaded by QEMU or copied onto a physical bootable USB drive.
