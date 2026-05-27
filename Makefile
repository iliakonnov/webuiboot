all:
	cargo build --target ./x86_64-unknown-uefi-debug.json
	mkdir -p esp/EFI/BOOT
	cp ./target/x86_64-unknown-uefi-debug/debug/webuiboot.efi esp/EFI/BOOT/BOOTX64.EFI

.PHONY: all