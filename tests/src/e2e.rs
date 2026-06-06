use webuiboot_e2e::QemuRunner;
use std::time::Duration;

#[test]
fn test_boot_windows_key_1() {
    let mut runner = QemuRunner::new("boot_windows_key_1");
    runner.prepare_esp(None);
    runner.start_qemu();

    // Wait for bootloader to start and initialize GUI
    assert!(
        runner.wait_for_serial("Starting Web", Duration::from_secs(12)),
        "Timeout waiting for bootloader start"
    );

    // Let the GUI load completely
    std::thread::sleep(Duration::from_secs(2));

    // Send key '1' (Kp 1 or standard 1, '1' in qemu is key 1)
    runner.send_key("1").expect("Failed to send key");

    // Wait for VM to shutdown on bootloader run
    let result = runner.wait_and_collect(Duration::from_secs(10));

    assert!(!result.timed_out, "QEMU timed out waiting for shutdown");
    assert!(
        result.serial_output.contains("BOOTED: \\EFI\\Microsoft\\Boot\\bootmgfw.efi"),
        "Expected Windows bootloader output, got:\n{}",
        result.serial_output
    );
}

#[test]
fn test_boot_linux_key_2() {
    let mut runner = QemuRunner::new("boot_linux_key_2");
    runner.prepare_esp(None);
    runner.start_qemu();

    assert!(
        runner.wait_for_serial("Starting Web", Duration::from_secs(12)),
        "Timeout waiting for bootloader start"
    );

    std::thread::sleep(Duration::from_secs(2));

    // Send key '2'
    runner.send_key("2").expect("Failed to send key");

    let result = runner.wait_and_collect(Duration::from_secs(10));

    assert!(!result.timed_out, "QEMU timed out waiting for shutdown");
    assert!(
        result.serial_output.contains("BOOTED: \\vmlinuz-test"),
        "Expected Linux bootloader output, got:\n{}",
        result.serial_output
    );
}

#[test]
fn test_boot_windows_arrow_enter() {
    let mut runner = QemuRunner::new("boot_windows_arrow_enter");
    runner.prepare_esp(None);
    runner.start_qemu();

    assert!(
        runner.wait_for_serial("Starting Web", Duration::from_secs(12)),
        "Timeout waiting for bootloader start"
    );

    std::thread::sleep(Duration::from_secs(2));

    // Default selection is Windows, so just press Enter ('ret' key in QEMU monitor)
    runner.send_key("ret").expect("Failed to send Enter key");

    let result = runner.wait_and_collect(Duration::from_secs(10));

    assert!(!result.timed_out, "QEMU timed out waiting for shutdown");
    assert!(
        result.serial_output.contains("BOOTED: \\EFI\\Microsoft\\Boot\\bootmgfw.efi"),
        "Expected Windows bootloader output, got:\n{}",
        result.serial_output
    );
}

#[test]
fn test_boot_linux_arrow_enter() {
    let mut runner = QemuRunner::new("boot_linux_arrow_enter");
    runner.prepare_esp(None);
    runner.start_qemu();

    assert!(
        runner.wait_for_serial("Starting Web", Duration::from_secs(12)),
        "Timeout waiting for bootloader start"
    );

    std::thread::sleep(Duration::from_secs(2));

    // Selection starts at 0 (Windows). Press Right to select Linux (1).
    runner.send_key("right").expect("Failed to send Right key");
    std::thread::sleep(Duration::from_millis(500));

    // Press Enter to boot selected Linux
    runner.send_key("ret").expect("Failed to send Enter key");

    let result = runner.wait_and_collect(Duration::from_secs(10));

    assert!(!result.timed_out, "QEMU timed out waiting for shutdown");
    assert!(
        result.serial_output.contains("BOOTED: \\vmlinuz-test"),
        "Expected Linux bootloader output, got:\n{}",
        result.serial_output
    );
}

#[test]
fn test_boot_windows_http() {
    let mut runner = QemuRunner::new("boot_windows_http");
    runner.prepare_esp(None);
    runner.start_qemu();

    // DHCP configured is needed for HTTP server
    assert!(
        runner.wait_for_serial("DHCP configured", Duration::from_secs(12)),
        "Timeout waiting for DHCP configuration"
    );

    std::thread::sleep(Duration::from_secs(1));

    // Send HTTP POST /boot/windows
    runner.send_http_boot("windows").expect("Failed to send HTTP boot request");

    let result = runner.wait_and_collect(Duration::from_secs(10));

    assert!(!result.timed_out, "QEMU timed out waiting for shutdown");
    assert!(
        result.serial_output.contains("BOOTED: \\EFI\\Microsoft\\Boot\\bootmgfw.efi"),
        "Expected Windows bootloader output via HTTP, got:\n{}",
        result.serial_output
    );
}

#[test]
fn test_boot_linux_http() {
    let mut runner = QemuRunner::new("boot_linux_http");
    runner.prepare_esp(None);
    runner.start_qemu();

    assert!(
        runner.wait_for_serial("DHCP configured", Duration::from_secs(12)),
        "Timeout waiting for DHCP configuration"
    );

    std::thread::sleep(Duration::from_secs(1));

    // Send HTTP POST /boot/linux
    runner.send_http_boot("linux").expect("Failed to send HTTP boot request");

    let result = runner.wait_and_collect(Duration::from_secs(10));

    assert!(!result.timed_out, "QEMU timed out waiting for shutdown");
    assert!(
        result.serial_output.contains("BOOTED: \\vmlinuz-test"),
        "Expected Linux bootloader output via HTTP, got:\n{}",
        result.serial_output
    );
}

#[test]
fn test_bootnext_single_entry() {
    let mut runner = QemuRunner::new("bootnext_single");
    // Place single Windows bootloader path in bootnext
    runner.prepare_esp(Some("\\EFI\\Microsoft\\Boot\\bootmgfw.efi\r\n"));
    runner.start_qemu();

    // Since bootnext is processed immediately at startup, it should boot directly
    let result = runner.wait_and_collect(Duration::from_secs(12));

    assert!(!result.timed_out, "QEMU timed out waiting for shutdown");
    assert!(
        result.serial_output.contains("BOOTED: \\EFI\\Microsoft\\Boot\\bootmgfw.efi"),
        "Expected Bootnext Windows boot, got:\n{}",
        result.serial_output
    );

    // Verify it popped the entry and deleted/updated the file
    assert!(
        result.serial_output.contains("No remaining bootloader paths") || 
        result.serial_output.contains("Removed empty bootnext"),
        "Expected bootnext file to be consumed or removed, got logs:\n{}",
        result.serial_output
    );
}

#[test]
fn test_bootnext_multi_entry() {
    let mut runner = QemuRunner::new("bootnext_multi");
    // Place multiple entries
    let content = "\\EFI\\Microsoft\\Boot\\bootmgfw.efi\r\n\\EFI\\other\\boot.efi\r\n";
    runner.prepare_esp(Some(content));
    runner.start_qemu();

    let result = runner.wait_and_collect(Duration::from_secs(12));

    assert!(!result.timed_out, "QEMU timed out waiting for shutdown");
    assert!(
        result.serial_output.contains("BOOTED: \\EFI\\Microsoft\\Boot\\bootmgfw.efi"),
        "Expected Bootnext first entry (Windows), got:\n{}",
        result.serial_output
    );
    assert!(
        result.serial_output.contains("Popped boot loader path"),
        "Expected log that path was popped, got:\n{}",
        result.serial_output
    );
}

#[test]
fn test_bootnext_fallback_on_failure() {
    let mut runner = QemuRunner::new("bootnext_fallback");
    // Write an invalid bootloader path
    runner.prepare_esp(Some("\\EFI\\nonexistent\\boot.efi\r\n"));
    runner.start_qemu();

    // It should try to load, fail, log the failure, and resume normal bootloader operations
    assert!(
        runner.wait_for_serial("Resuming normal bootloader", Duration::from_secs(12)),
        "Timeout waiting for fallback message"
    );

    // After fallback, we should be at the GUI. Let's press '1' to boot Windows.
    std::thread::sleep(Duration::from_secs(2));
    runner.send_key("1").expect("Failed to send key 1");

    let result = runner.wait_and_collect(Duration::from_secs(10));

    assert!(!result.timed_out, "QEMU timed out waiting for shutdown");
    assert!(
        result.serial_output.contains("BOOTED: \\EFI\\Microsoft\\Boot\\bootmgfw.efi"),
        "Expected Windows boot after fallback, got:\n{}",
        result.serial_output
    );
}

#[test]
fn test_bootnext_systemd_entry() {
    let mut runner = QemuRunner::new("bootnext_systemd");
    // Place a systemd entry path in bootnext
    runner.prepare_esp(Some("\\loader\\entries\\test.conf\r\n"));
    runner.start_qemu();

    // Since bootnext is processed immediately at startup, it should boot directly
    let result = runner.wait_and_collect(Duration::from_secs(12));

    assert!(!result.timed_out, "QEMU timed out waiting for shutdown");
    assert!(
        result.serial_output.contains("BOOTED: \\vmlinuz-test"),
        "Expected Bootnext Linux systemd boot, got:\n{}",
        result.serial_output
    );

    // Verify it popped the entry and deleted/updated the file
    assert!(
        result.serial_output.contains("No remaining bootloader paths") || 
        result.serial_output.contains("Removed empty bootnext"),
        "Expected bootnext file to be consumed or removed, got logs:\n{}",
        result.serial_output
    );
}

#[test]
fn test_bootnext_systemd_entry_forward_slash() {
    let mut runner = QemuRunner::new("bootnext_systemd_forward");
    // Place a systemd entry path with forward slashes in bootnext
    runner.prepare_esp(Some("/loader/entries/test.conf\r\n"));
    runner.start_qemu();

    // Since bootnext is processed immediately at startup, it should boot directly
    let result = runner.wait_and_collect(Duration::from_secs(12));

    assert!(!result.timed_out, "QEMU timed out waiting for shutdown");
    assert!(
        result.serial_output.contains("BOOTED: \\vmlinuz-test"),
        "Expected Bootnext Linux systemd boot, got:\n{}",
        result.serial_output
    );
}
