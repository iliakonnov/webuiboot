#![no_std]
#![no_main]

extern crate alloc;

mod boot;
mod net;
mod web;
mod slint_plat;

use log::{info, warn};
use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::network::snp::SimpleNetwork;
use alloc::boxed::Box;

use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpSocketBuffer};
use smoltcp::socket::dhcpv4::{Socket as Dhcpv4Socket, Event as Dhcpv4Event};
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr, Ipv4Address, Ipv4Cidr};
use smoltcp::time::Instant;

slint::include_modules!();

struct GlobalNet {
    iface: Interface,
    device: net::UefiSnpDevice,
    sockets: SocketSet<'static>,
    dhcp_handle: smoltcp::iface::SocketHandle,
    tcp_handle: smoltcp::iface::SocketHandle,
    ms_elapsed: u64,
}

static mut GLOBAL_NET: Option<GlobalNet> = None;
static mut APP_WINDOW: Option<slint::Weak<AppWindow>> = None;
static mut PENDING_LOGS: Option<alloc::string::String> = None;

struct SlintLogger;

impl log::Log for SlintLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let msg = alloc::format!("[{:5}]: {}\n", record.level().as_str(), record.args());

        // Print to UEFI Stdout (console/serial port)
        use core::fmt::Write;
        let _ = uefi::system::with_stdout(|stdout| {
            let _ = stdout.write_str(&msg);
        });

        unsafe {
            let app_ptr = core::ptr::addr_of!(APP_WINDOW);
            if let Some(ref weak) = *app_ptr {
                if let Some(window) = weak.upgrade() {
                    let pending_ptr = core::ptr::addr_of_mut!(PENDING_LOGS);
                    let mut current_logs = alloc::string::String::from(window.get_log_text().as_str());
                    if let Some(pending) = (*pending_ptr).take() {
                        current_logs.push_str(&pending);
                    }
                    current_logs.push_str(&msg);

                    // Truncate logs if they get too long (keep last 4000 characters)
                    if current_logs.len() > 4000 {
                        let drain_index = current_logs.len() - 4000;
                        if let Some(next_nl) = current_logs[drain_index..].find('\n') {
                            current_logs = alloc::string::String::from(&current_logs[drain_index + next_nl + 1..]);
                        } else {
                            current_logs = alloc::string::String::from(&current_logs[drain_index..]);
                        }
                    }

                    window.set_log_text(current_logs.into());
                    crate::slint_plat::force_flush_logs();
                    return;
                }
            }

            let pending_ptr = core::ptr::addr_of_mut!(PENDING_LOGS);
            if (*pending_ptr).is_none() {
                *pending_ptr = Some(alloc::string::String::new());
            }
            if let Some(ref mut pending) = *pending_ptr {
                pending.push_str(&msg);
                if pending.len() > 4000 {
                    let drain_index = pending.len() - 4000;
                    if let Some(next_nl) = pending[drain_index..].find('\n') {
                        *pending = alloc::string::String::from(&pending[drain_index + next_nl + 1..]);
                    } else {
                        *pending = alloc::string::String::from(&pending[drain_index..]);
                    }
                }
            }
        }
    }

    fn flush(&self) {}
}

static LOGGER: SlintLogger = SlintLogger;


pub fn update_slint_ip(ip: &str) {
    unsafe {
        let app_ptr = core::ptr::addr_of!(APP_WINDOW);
        if let Some(ref weak) = *app_ptr {
            if let Some(window) = weak.upgrade() {
                window.set_ip_address(ip.into());
            }
        }
    }
}

pub fn poll_network_from_slint(ip_out: &mut Option<alloc::string::String>) -> Option<crate::web::BootSelection> {
    let net_ptr = core::ptr::addr_of_mut!(GLOBAL_NET);
    let net = unsafe { (*net_ptr).as_mut()? };
    
    net.ms_elapsed += 5; // wait_for_input waits roughly 5ms
    let timestamp = Instant::from_millis(net.ms_elapsed as i64);
    
    net.iface.poll(timestamp, &mut net.device, &mut net.sockets);

    // Process DHCP
    let dhcp_event = net.sockets.get_mut::<Dhcpv4Socket>(net.dhcp_handle).poll();
    if let Some(Dhcpv4Event::Configured(config)) = dhcp_event {
        info!("DHCP configured!");
        info!("IP: {}", config.address);
        net.iface.update_ip_addrs(|ip_addrs| {
            ip_addrs.clear();
            ip_addrs.push(IpCidr::Ipv4(config.address)).unwrap();
        });
        if let Some(router) = config.router {
            let _ = net.iface.routes_mut().add_default_ipv4_route(router);
        }
        *ip_out = Some(alloc::format!("{}", config.address));
    }

    // Process TCP
    let tcp_socket = net.sockets.get_mut::<TcpSocket>(net.tcp_handle);
    if !tcp_socket.is_open() {
        let _ = tcp_socket.listen(80);
    }

    if tcp_socket.can_recv() {
        let mut request_buffer = [0u8; 1024];
        if let Ok(size_read) = tcp_socket.recv_slice(&mut request_buffer) {
            if size_read > 0 {
                if let Some((response, boot_sel)) = web::handle_http_request(&request_buffer[..size_read]) {
                    let _ = tcp_socket.send_slice(response.as_bytes());
                    tcp_socket.close();
                    return boot_sel; // Return remote boot request!
                }
            }
        }
    }
    
    None
}

fn wait_for_gdb() {
    let loaded_image = uefi::boot::open_protocol_exclusive::<LoadedImage>(uefi::boot::image_handle()).unwrap();

    let base_addr = loaded_image.info().0;
    info!("DEBUGGER: Loaded at {:#x}", base_addr as u64);

    unsafe {
        core::ptr::write_volatile(0x10008 as *mut u64, base_addr as u64);
        core::ptr::write_volatile(0x10000 as *mut u64, 0xDEADBEEF);
    }

    #[unsafe(no_mangle)]
    static mut GDB_ATTACHED: usize = 0;

    unsafe {
        while core::ptr::read_volatile(&raw mut GDB_ATTACHED) == 0 {
            core::hint::spin_loop();
        }
    }
    info!("Debugger attached, resuming execution...");
}

#[entry]
fn efi_main() -> Status {
    uefi::helpers::init().expect("Failed to initialize UEFI runtime");

    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);

    #[cfg(debug_assertions)]
    wait_for_gdb(); // Uncomment to debug

    info!("Starting Web & Slint UI Bootloader...");

    // Setup network interface
    let snp_handle = match uefi::boot::find_handles::<SimpleNetwork>() {
        Ok(handles) if !handles.is_empty() => handles[0],
        _ => {
            warn!("No network interface found!");
            return Status::UNSUPPORTED;
        }
    };

    let snp_protocol = uefi::boot::open_protocol_exclusive::<SimpleNetwork>(snp_handle)
        .expect("Failed to open SimpleNetwork protocol");

    let mut device = net::UefiSnpDevice::new(snp_protocol);
    let mac = device.mac_address();
    info!("MAC Address: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
          mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);

    let hw_addr = HardwareAddress::Ethernet(EthernetAddress(mac));
    let mut config = Config::new(hw_addr);
    config.random_seed = 0x12345678;

    let mut iface = Interface::new(config, &mut device, Instant::from_millis(0));
    
    // Initial static IP
    iface.update_ip_addrs(|ip_addrs| {
        ip_addrs.push(IpCidr::Ipv4(Ipv4Cidr::new(Ipv4Address::new(192, 168, 1, 30), 24))).unwrap();
    });

    let mut sockets = SocketSet::new(alloc::vec![]);

    let dhcp_socket = Dhcpv4Socket::new();
    let dhcp_handle = sockets.add(dhcp_socket);

    // Leak buffers to get 'static lifetime
    let rx_buf_slice = alloc::vec![0u8; 4096];
    let tx_buf_slice = alloc::vec![0u8; 4096];
    let rx_leak = Box::leak(rx_buf_slice.into_boxed_slice());
    let tx_leak = Box::leak(tx_buf_slice.into_boxed_slice());

    let tcp_rx_buffer = TcpSocketBuffer::new(rx_leak);
    let tcp_tx_buffer = TcpSocketBuffer::new(tx_leak);
    let tcp_socket = TcpSocket::new(tcp_rx_buffer, tcp_tx_buffer);
    let tcp_handle = sockets.add(tcp_socket);

    unsafe {
        GLOBAL_NET = Some(GlobalNet {
            iface,
            device,
            sockets,
            dhcp_handle,
            tcp_handle,
            ms_elapsed: 0,
        });
    }

    // Set up Slint Platform
    slint::platform::set_platform(Box::<slint_plat::Platform>::default()).unwrap();

    // Create App UI
    let ui = AppWindow::new().unwrap();
    
    // Initial UI Values
    ui.set_ip_address("192.168.1.30".into());
    ui.set_status_text("Select OS to boot...".into());

    unsafe {
        APP_WINDOW = Some(ui.as_weak());
        let pending_ptr = core::ptr::addr_of_mut!(PENDING_LOGS);
        if let Some(pending) = (*pending_ptr).take() {
            ui.set_log_text(pending.into());
        }
    }

    // Setup Callbacks
    ui.on_boot_windows(|| {
        boot::boot_os("\\EFI\\Microsoft\\Boot\\bootmgfw.efi");
    });

    ui.on_boot_linux(|| {
        boot::boot_linux_direct();
    });

    // Run Slint Event Loop (blocks)
    ui.run().unwrap();

    Status::SUCCESS
}
