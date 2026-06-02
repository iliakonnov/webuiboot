ifeq ($(OS),Windows_NT)
    MKDIR = if not exist "esp\EFI\BOOT" mkdir "esp\EFI\BOOT"
    CP = copy /Y "target\x86_64-unknown-uefi-debug\debug\webuiboot.efi" "esp\EFI\BOOT\BOOTX64.EFI"
else
    MKDIR = mkdir -p esp/EFI/BOOT
    CP = cp ./target/x86_64-unknown-uefi-debug/debug/webuiboot.efi esp/EFI/BOOT/BOOTX64.EFI
endif

all:
	cargo build -p webuiboot --target ./x86_64-unknown-uefi-debug.json -Z build-std=core,compiler_builtins,alloc
	$(MKDIR)
	$(CP)

.PHONY: all