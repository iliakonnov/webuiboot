use log::{error, info};
use uefi::boot::LoadImageSource;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, Directory};
use uefi::proto::media::fs::SimpleFileSystem;

fn open_protocol_exclusive<P: uefi::proto::ProtocolPointer + ?Sized>(
    handle: uefi::Handle,
) -> Result<uefi::boot::ScopedProtocol<P>, uefi::Status> {
    uefi::boot::open_protocol_exclusive::<P>(handle).map_err(|e| e.status())
}

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

fn boot_linux_from_entry_file(
    root: &mut Directory,
    fs_handle: uefi::Handle,
    entry_path_str: &str,
) -> Result<(), uefi::Status> {
    let entry_path = uefi::CString16::try_from(entry_path_str)
        .map_err(|_| uefi::Status::INVALID_PARAMETER)?;

    let file_handle = root
        .open(&entry_path, FileMode::Read, FileAttribute::empty())
        .map_err(|e| e.status())?;

    let mut regular_file = file_handle
        .into_regular_file()
        .ok_or(uefi::Status::LOAD_ERROR)?;

    let info = regular_file
        .get_boxed_info::<FileInfo>()
        .map_err(|e| e.status())?;

    let size = info.file_size() as usize;
    let mut buffer = alloc::vec![0u8; size];
    regular_file
        .read(&mut buffer)
        .map_err(|e| e.status())?;

    let content_str = core::str::from_utf8(&buffer)
        .map_err(|_| uefi::Status::LOAD_ERROR)?;

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
            info!("Entry {} is not a Linux entry (no 'linux' key). Skipping.", entry_path_str);
            return Err(uefi::Status::NOT_FOUND);
        }
    };

    info!("Found Linux boot entry: {}", entry_path_str);

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

    let linux_cstr = uefi::CString16::try_from(linux_path.as_str())
        .map_err(|_| uefi::Status::INVALID_PARAMETER)?;

    let kernel_handle = root
        .open(&linux_cstr, FileMode::Read, FileAttribute::empty())
        .map_err(|e| e.status())?;

    let mut kernel_file = kernel_handle
        .into_regular_file()
        .ok_or(uefi::Status::LOAD_ERROR)?;

    let k_info = kernel_file
        .get_boxed_info::<FileInfo>()
        .map_err(|e| e.status())?;

    let k_size = k_info.file_size() as usize;
    let mut k_buffer = alloc::vec![0u8; k_size];
    kernel_file
        .read(&mut k_buffer)
        .map_err(|e| e.status())?;

    let device_path = open_protocol_exclusive::<uefi::proto::device_path::DevicePath>(fs_handle)?;

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
            return Err(e.status());
        }
    };

    let mut loaded_image = match uefi::boot::open_protocol_exclusive::<uefi::proto::loaded_image::LoadedImage>(loaded_os) {
        Ok(li) => li,
        Err(e) => {
            error!("Failed to open LoadedImage protocol: {:?}", e);
            let _ = uefi::boot::unload_image(loaded_os);
            return Err(e.status());
        }
    };

    let options_cstr16 = match uefi::CString16::try_from(final_options.as_str()) {
        Ok(c) => c,
        Err(_) => {
            let _ = uefi::boot::unload_image(loaded_os);
            return Err(uefi::Status::INVALID_PARAMETER);
        }
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
        let _ = uefi::boot::unload_image(loaded_os);
        return Err(e.status());
    }

    Ok(())
}

pub fn boot_linux_direct() {
    info!("Attempting to boot Linux directly...");

    let fs_handles = match uefi::boot::find_handles::<SimpleFileSystem>() {
        Ok(h) => h,
        Err(_) => return,
    };

    for handle in fs_handles {
        let mut fs = match open_protocol_exclusive::<SimpleFileSystem>(handle) {
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
            if let Err(e) = boot_linux_from_entry_file(&mut root, handle, &entry_path_str) {
                error!("Failed to boot Linux entry {}: {:?}", entry_path_str, e);
                continue;
            }
        }
    }

    error!("Could not find or directly boot Linux");
}

pub fn boot_os(path: &str) -> Result<(), uefi::Status> {
    info!("Attempting to boot: {}", path);

    let fs_handles = match uefi::boot::find_handles::<SimpleFileSystem>() {
        Ok(h) => h,
        Err(e) => return Err(e.status()),
    };

    for handle in fs_handles {
        let mut fs = match open_protocol_exclusive::<SimpleFileSystem>(handle) {
            Ok(fs) => fs,
            Err(_) => continue,
        };

        let mut root = match fs.open_volume() {
            Ok(root) => root,
            Err(_) => continue,
        };

        let is_systemd_entry = path.ends_with(".conf") || path.ends_with(".CONF");
        if is_systemd_entry {
            if let Err(e) = boot_linux_from_entry_file(&mut root, handle, path) {
                error!("Failed to boot Linux entry {}: {:?}", path, e);
                continue;
            }
            return Ok(());
        }

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

        let device_path = match open_protocol_exclusive::<uefi::proto::device_path::DevicePath>(handle) {
            Ok(dp) => dp,
            Err(_) => continue,
        };

        info!("Loading OS image into memory...");
        let loaded_os = match uefi::boot::load_image(
            uefi::boot::image_handle(),
            LoadImageSource::FromBuffer {
                buffer: &buffer,
                file_path: Some(&device_path),
            },
        ) {
            Ok(img) => img,
            Err(e) => {
                error!("Failed to load OS image: {:?}", e);
                continue;
            }
        };

        info!("Starting OS. Exiting bootloader control...");
        crate::slint_plat::force_flush_logs();
        uefi::boot::stall(core::time::Duration::from_millis(1500));
        // Reset the console to restore standard text mode for systemd-boot or other loaders
        let _ = uefi::system::with_stdout(|stdout| {
            let _ = stdout.reset(false);
        });
        
        if let Err(e) = uefi::boot::start_image(loaded_os) {
            error!("Failed to start OS: {:?}", e);
            let _ = uefi::boot::unload_image(loaded_os);
            continue;
        }

        return Ok(());
    }

    error!("Could not find or boot {}", path);
    Err(uefi::Status::NOT_FOUND)
}

pub fn check_and_process_bootnext() {
    info!("Checking for bootnext flag file...");

    let fs_handles = match uefi::boot::find_handles::<SimpleFileSystem>() {
        Ok(h) => h,
        Err(e) => {
            info!("Failed to find SimpleFileSystem handles: {:?}", e);
            return;
        }
    };

    for handle in fs_handles {
        let mut target_path_str = None;
        let mut processed = false;

        {
            let mut fs = match open_protocol_exclusive::<SimpleFileSystem>(handle) {
                Ok(fs) => fs,
                Err(_) => continue,
            };

            let mut root = match fs.open_volume() {
                Ok(root) => root,
                Err(_) => continue,
            };

            let flag_filenames = &["\\bootnext.txt", "\\EFI\\bootnext.txt"];

            for flag_filename in flag_filenames {
                let flag_cstr16 = match uefi::CString16::try_from(*flag_filename) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                // Try to open flag file for Read/Write to be able to modify or delete it
                let file_handle = match root.open(&flag_cstr16, FileMode::ReadWrite, FileAttribute::empty()) {
                    Ok(f) => f,
                    Err(_) => continue,
                };

                let mut regular_file = match file_handle.into_regular_file() {
                    Some(f) => f,
                    None => continue,
                };

                info!("Found bootnext flag file: {}", flag_filename);
                processed = true;

                let info = match regular_file.get_boxed_info::<FileInfo>() {
                    Ok(info) => info,
                    Err(_) => {
                        error!("Failed to get bootnext file info");
                        break;
                    }
                };

                let size = info.file_size() as usize;
                let mut buffer = alloc::vec![0u8; size];

                if let Err(e) = regular_file.read(&mut buffer) {
                    error!("Failed to read bootnext flag file: {:?}", e);
                    break;
                }

                let content_str = match core::str::from_utf8(&buffer) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("bootnext file content is not valid UTF-8: {:?}", e);
                        break;
                    }
                };

                // Handle BOM (byte order mark) if present
                let trimmed_content = content_str.strip_prefix("\u{feff}").unwrap_or(content_str);

                // Parse lines, skip empty lines
                let mut lines = trimmed_content.lines().map(|line| line.trim()).filter(|line| !line.is_empty());
                let first_line = lines.next();

                if let Some(target_path) = first_line {
                    target_path_str = Some(alloc::string::String::from(target_path));
                    let remaining_lines: alloc::vec::Vec<&str> = lines.collect();

                    info!("Popped boot loader path: {}", target_path_str.as_ref().unwrap());

                    // Delete the old file
                    match regular_file.delete() {
                        Ok(_) => {
                            info!("Deleted old bootnext file.");
                        }
                        Err(_) => {
                            error!("Failed to delete old bootnext file.");
                        }
                    }

                    // Write remaining lines back if not empty
                    if !remaining_lines.is_empty() {
                        // Recreate the file empty
                        let new_handle = match root.open(&flag_cstr16, FileMode::CreateReadWrite, FileAttribute::empty()) {
                            Ok(h) => h,
                            Err(e) => {
                                error!("Failed to recreate bootnext file: {:?}", e);
                                break;
                            }
                        };

                        let mut new_file = match new_handle.into_regular_file() {
                            Some(f) => f,
                            None => break,
                        };

                        let mut new_content = alloc::string::String::new();
                        for (i, line) in remaining_lines.iter().enumerate() {
                            if i > 0 {
                                new_content.push_str("\r\n");
                            }
                            new_content.push_str(line);
                        }
                        new_content.push_str("\r\n");

                        if let Err(e) = new_file.write(new_content.as_bytes()) {
                            error!("Failed to write updated bootnext file: {:?}", e);
                        }
                        let _ = new_file.flush();
                    } else {
                        info!("No remaining bootloader paths. bootnext file removed.");
                    }
                } else {
                    // File is empty, just remove it from disk
                    info!("bootnext file is empty. Removing it from disk.");
                    match regular_file.delete() {
                        Ok(_) => {
                            info!("Removed empty bootnext file.");
                        }
                        Err(_) => {
                            error!("Failed to remove empty bootnext file.");
                        }
                    }
                }

                break;
            }
        } // `fs` and `root` are dropped/closed here!

        if processed {
            if let Some(target_path_str) = target_path_str {
                // Try to boot the popped path
                info!("Attempting immediate boot of: {}", target_path_str);
                let boot_path = target_path_str.replace('/', "\\");
                if let Err(e) = boot_os(&boot_path) {
                    error!("Immediate boot failed: {:?}", e);
                    info!("Resuming normal bootloader operations.");
                }
            }
            break;
        }
    }
}

