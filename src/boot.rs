use log::{error, info};
use uefi::boot::LoadImageSource;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, Directory};
use uefi::proto::media::fs::SimpleFileSystem;

fn find_all_entries(root: &mut Directory) -> Option<alloc::vec::Vec<alloc::string::String>> {
    let entries_path = uefi::CString16::try_from("\\loader\\entries").ok()?;
    let mut entries_dir = match root.open(&entries_path, FileMode::Read, FileAttribute::DIRECTORY) {
        Ok(f) => match f.into_type() {
            Ok(uefi::proto::media::file::FileType::Dir(d)) => d,
            _ => return None,
        },
        Err(_) => return None,
    };

    let mut buffer = alloc::vec![0u8; 1024];
    let mut entries = alloc::vec::Vec::new();

    loop {
        match entries_dir.read_entry(&mut buffer) {
            Ok(Some(info)) => {
                let name = info.file_name();
                let name_str = alloc::format!("{}", name);
                if name_str.ends_with(".conf") {
                    entries.push(name_str);
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    Some(entries)
}

pub fn boot_linux_direct() {
    info!("Attempting to boot Linux directly...");

    let fs_handles = match uefi::boot::find_handles::<SimpleFileSystem>() {
        Ok(h) => h,
        Err(_) => return,
    };

    for handle in fs_handles {
        let mut fs = match uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(handle) {
            Ok(fs) => fs,
            Err(_) => continue,
        };

        let mut root = match fs.open_volume() {
            Ok(root) => root,
            Err(_) => continue,
        };

        let entries = match find_all_entries(&mut root) {
            Some(e) => e,
            None => continue,
        };

        let mut sorted_entries = entries;
        sorted_entries.sort_by(|a, b| {
            let a_is_fallback = a.contains("fallback");
            let b_is_fallback = b.contains("fallback");
            a_is_fallback.cmp(&b_is_fallback)
        });

        for entry_filename in sorted_entries {
            let entry_path_str = alloc::format!("\\loader\\entries\\{}", entry_filename);
            let entry_path = match uefi::CString16::try_from(entry_path_str.as_str()) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let file_handle = match root.open(&entry_path, FileMode::Read, FileAttribute::empty()) {
                Ok(f) => f,
                Err(_) => continue,
            };

            let mut regular_file = match file_handle.into_regular_file() {
                Some(f) => f,
                None => continue,
            };

            let info = match regular_file.get_boxed_info::<FileInfo>() {
                Ok(info) => info,
                Err(_) => continue,
            };

            let size = info.file_size() as usize;
            let mut buffer = alloc::vec![0u8; size];
            if regular_file.read(&mut buffer).is_err() {
                continue;
            }

            let content_str = match core::str::from_utf8(&buffer) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let mut linux_path = None;
            let mut initrd_paths = alloc::vec::Vec::new();
            let mut options_str = None;

            for line in content_str.lines() {
                let line = line.trim();
                if line.starts_with('#') || line.is_empty() {
                    continue;
                }

                let mut parts = line.split_whitespace();
                let key = match parts.next() {
                    Some(k) => k,
                    None => continue,
                };

                let value = parts.collect::<alloc::vec::Vec<&str>>().join(" ");
                if value.is_empty() {
                    continue;
                }

                match key {
                    "linux" => linux_path = Some(alloc::string::String::from(value)),
                    "initrd" => initrd_paths.push(alloc::string::String::from(value)),
                    "options" => options_str = Some(alloc::string::String::from(value)),
                    _ => {}
                }
            }

            let linux_val = match linux_path {
                Some(p) => p,
                None => {
                    info!("Entry {} is not a Linux entry (no 'linux' key). Skipping.", entry_filename);
                    continue;
                }
            };

            info!("Found Linux boot entry: {}", entry_filename);

            let linux_path = linux_val.replace('/', "\\");

            let mut final_options = options_str.unwrap_or_else(|| alloc::string::String::new());
            for initrd in initrd_paths {
                let win_initrd = initrd.replace('/', "\\");
                if !final_options.is_empty() {
                    final_options.push(' ');
                }
                final_options.push_str("initrd=");
                final_options.push_str(&win_initrd);
            }

            info!("Linux path: {}", linux_path);
            info!("Kernel options: {}", final_options);

            let linux_cstr = match uefi::CString16::try_from(linux_path.as_str()) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let kernel_handle = match root.open(&linux_cstr, FileMode::Read, FileAttribute::empty()) {
                Ok(f) => f,
                Err(_) => continue,
            };

            let mut kernel_file = match kernel_handle.into_regular_file() {
                Some(f) => f,
                None => continue,
            };

            let k_info = match kernel_file.get_boxed_info::<FileInfo>() {
                Ok(info) => info,
                Err(_) => continue,
            };

            let k_size = k_info.file_size() as usize;
            let mut k_buffer = alloc::vec![0u8; k_size];
            if kernel_file.read(&mut k_buffer).is_err() {
                continue;
            }

            let device_path = match uefi::boot::open_protocol_exclusive::<uefi::proto::device_path::DevicePath>(handle) {
                Ok(dp) => dp,
                Err(_) => continue,
            };

            info!("Loading Linux kernel EFI Stub into memory...");
            let loaded_os = match uefi::boot::load_image(
                uefi::boot::image_handle(),
                LoadImageSource::FromBuffer {
                    buffer: &k_buffer,
                    file_path: Some(&device_path),
                },
            ) {
                Ok(img) => img,
                Err(e) => {
                    error!("Failed to load Linux kernel image: {:?}", e);
                    continue;
                }
            };

            let mut loaded_image = match uefi::boot::open_protocol_exclusive::<uefi::proto::loaded_image::LoadedImage>(loaded_os) {
                Ok(li) => li,
                Err(e) => {
                    error!("Failed to open LoadedImage protocol: {:?}", e);
                    continue;
                }
            };

            let options_cstr16 = match uefi::CString16::try_from(final_options.as_str()) {
                Ok(c) => c,
                Err(_) => continue,
            };

            unsafe {
                let slice = options_cstr16.as_slice_with_nul();
                loaded_image.set_load_options(
                    slice.as_ptr() as *const u8,
                    (slice.len() * 2) as u32,
                );
            }

            info!("Starting Linux kernel directly. Exiting bootloader control...");
            crate::slint_plat::force_flush_logs();
            uefi::boot::stall(core::time::Duration::from_millis(1500));
            let _ = uefi::system::with_stdout(|stdout| {
                let _ = stdout.reset(false);
            });
            
            if let Err(e) = uefi::boot::start_image(loaded_os) {
                error!("Failed to start Linux kernel image: {:?}", e);
                continue;
            }
        }
    }

    error!("Could not find or directly boot Linux");
}

pub fn boot_os(path: &str) {
    info!("Attempting to boot: {}", path);

    let fs_handles = match uefi::boot::find_handles::<SimpleFileSystem>() {
        Ok(h) => h,
        Err(_) => return,
    };

    for handle in fs_handles {
        let mut fs = match uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(handle) {
            Ok(fs) => fs,
            Err(_) => continue,
        };

        let mut root = match fs.open_volume() {
            Ok(root) => root,
            Err(_) => continue,
        };

        let cstr16 = match uefi::CString16::try_from(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let file_handle = match root.open(&cstr16, FileMode::Read, FileAttribute::empty()) {
            Ok(f) => f,
            Err(_) => continue,
        };

        let mut regular_file = match file_handle.into_regular_file() {
            Some(f) => f,
            None => {
                error!("Found path {}, but it's not a regular file", path);
                continue;
            }
        };

        let info = match regular_file.get_boxed_info::<FileInfo>() {
            Ok(info) => info,
            Err(_) => continue,
        };

        let size = info.file_size() as usize;
        let mut buffer = alloc::vec![0u8; size];

        match regular_file.read(&mut buffer) {
            Ok(_) => {
                info!("Loaded {} bytes from {}", size, path);
            }
            Err(e) => {
                error!("Failed to read file: {:?}", e);
                continue;
            }
        }

        let device_path = match uefi::boot::open_protocol_exclusive::<uefi::proto::device_path::DevicePath>(handle) {
            Ok(dp) => dp,
            Err(_) => continue,
        };

        info!("Loading OS image into memory...");
        let loaded_os = uefi::boot::load_image(
            uefi::boot::image_handle(),
            LoadImageSource::FromBuffer {
                buffer: &buffer,
                file_path: Some(&device_path),
            },
        )
        .expect("Failed to load OS image");

        info!("Starting OS. Exiting bootloader control...");
        crate::slint_plat::force_flush_logs();
        uefi::boot::stall(core::time::Duration::from_millis(1500));
        // Reset the console to restore standard text mode for systemd-boot or other loaders
        let _ = uefi::system::with_stdout(|stdout| {
            let _ = stdout.reset(false);
        });
        uefi::boot::start_image(loaded_os).expect("Failed to start OS");
    }

    error!("Could not find or boot {}", path);
}
