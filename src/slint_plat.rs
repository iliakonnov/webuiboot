use alloc::rc::Rc;
use core::slice;
use core::time::Duration;
use log::info;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::platform::software_renderer;
use uefi::Char16;
use uefi::boot::ScopedProtocol;
use uefi::proto::console::{gop::{BltPixel, GraphicsOutput, BltOp, BltRegion}, pointer::Pointer};

static mut MOUSE_POINTERS: alloc::vec::Vec<ScopedProtocol<Pointer>> = alloc::vec::Vec::new();
static mut GLOBAL_GOP: Option<*mut GraphicsOutput> = None;
static mut GLOBAL_FB: Option<*mut [SlintBltPixel]> = None;
static mut GLOBAL_WINDOW: Option<alloc::rc::Weak<software_renderer::MinimalSoftwareWindow>> = None;
static mut IN_FLUSH: bool = false;

const POINTER_WIDTH: usize = 12;
const POINTER_HEIGHT: usize = 19;
// Standard cursor representation (1 = white, 2 = black border, 0 = transparent)
const POINTER_PIXELS: [u8; 12 * 19] = [
    2,0,0,0,0,0,0,0,0,0,0,0,
    2,2,0,0,0,0,0,0,0,0,0,0,
    2,1,2,0,0,0,0,0,0,0,0,0,
    2,1,1,2,0,0,0,0,0,0,0,0,
    2,1,1,1,2,0,0,0,0,0,0,0,
    2,1,1,1,1,2,0,0,0,0,0,0,
    2,1,1,1,1,1,2,0,0,0,0,0,
    2,1,1,1,1,1,1,2,0,0,0,0,
    2,1,1,1,1,1,1,1,2,0,0,0,
    2,1,1,1,1,1,1,1,1,2,0,0,
    2,1,1,1,1,1,2,2,2,2,2,2,
    2,1,1,2,1,1,2,0,0,0,0,0,
    2,1,2,0,2,1,1,2,0,0,0,0,
    2,2,0,0,2,1,1,2,0,0,0,0,
    0,0,0,0,0,2,1,1,2,0,0,0,
    0,0,0,0,0,2,1,1,2,0,0,0,
    0,0,0,0,0,0,2,2,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,
];

fn timer_tick() -> u64 {
    #[cfg(target_arch = "x86")]
    unsafe {
        core::arch::x86::_rdtsc()
    }

    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::x86_64::_rdtsc()
    }
}

fn timer_freq() -> u64 {
    let start = timer_tick();
    uefi::boot::stall(core::time::Duration::from_millis(1));
    let end = timer_tick();
    (end - start) * 1000
}

fn pointer_init() {
    unsafe {
        let ptrs = core::ptr::addr_of_mut!(MOUSE_POINTERS);
        if let Ok(handles) = uefi::boot::find_handles::<Pointer>() {
            for handle in handles {
                if let Ok(mut pointer) = uefi::boot::open_protocol_exclusive::<Pointer>(handle) {
                    let _ = pointer.reset(false);
                    let mode = *pointer.mode();
                    info!("Mouse pointer initialized: {:?}, resolution: {:?}, buttons: {:?}", handle, mode.resolution, mode.has_button);
                    (*ptrs).push(pointer);
                }
            }
        }
    }
}

fn get_key_press() -> Option<char> {
    use slint::platform::Key::*;
    use uefi::proto::console::text::Key as UefiKey;
    use uefi::proto::console::text::ScanCode as Scan;

    let nl = Char16::try_from('\r').unwrap();
    let tab_char = Char16::try_from('\t').unwrap();

    let key_res = uefi::system::with_stdin(|stdin| stdin.read_key());
    /*
    if let Ok(Some(ref key)) = key_res {
        info!("Raw UEFI key read: {:?}", key);
    }
    */

    match key_res {
        Err(_) | Ok(None) => None,
        Ok(Some(UefiKey::Printable(key))) if key == nl => Some('\n'),
        Ok(Some(UefiKey::Printable(key))) if key == tab_char => Some(char::from(Tab)),
        Ok(Some(UefiKey::Printable(key))) => Some(char::from(key)),
        Ok(Some(UefiKey::Special(key))) => Some(
            match key {
                Scan::UP => UpArrow,
                Scan::DOWN => DownArrow,
                Scan::RIGHT => RightArrow,
                Scan::LEFT => LeftArrow,
                Scan::HOME => Home,
                Scan::END => End,
                Scan::INSERT => Insert,
                Scan::DELETE => Delete,
                Scan::PAGE_UP => PageUp,
                Scan::PAGE_DOWN => PageDown,
                Scan::ESCAPE => Escape,
                Scan::FUNCTION_1 => F1,
                Scan::FUNCTION_2 => F2,
                Scan::FUNCTION_3 => F3,
                Scan::FUNCTION_4 => F4,
                Scan::FUNCTION_5 => F5,
                Scan::FUNCTION_6 => F6,
                Scan::FUNCTION_7 => F7,
                Scan::FUNCTION_8 => F8,
                Scan::FUNCTION_9 => F9,
                Scan::FUNCTION_10 => F10,
                Scan::FUNCTION_11 => F11,
                Scan::FUNCTION_12 => F12,
                _ => return None,
            }
            .into(),
        ),
    }
}

fn wait_for_input(max_timeout: Option<Duration>) {
    let timeout = Duration::from_millis(5).min(max_timeout.unwrap_or(Duration::from_millis(5)));
    uefi::boot::stall(timeout);
}

#[repr(transparent)]
#[derive(Clone, Copy)]
struct SlintBltPixel(BltPixel);

impl software_renderer::TargetPixel for SlintBltPixel {
    fn blend(&mut self, color: software_renderer::PremultipliedRgbaColor) {
        let a = (u8::MAX - color.alpha) as u16;
        self.0.red = (self.0.red as u16 * a / 255) as u8 + color.red;
        self.0.green = (self.0.green as u16 * a / 255) as u8 + color.green;
        self.0.blue = (self.0.blue as u16 * a / 255) as u8 + color.blue;
    }

    fn from_rgb(red: u8, green: u8, blue: u8) -> Self {
        SlintBltPixel(BltPixel::new(red, green, blue))
    }
}

pub struct Platform {
    window: Rc<software_renderer::MinimalSoftwareWindow>,
    timer_freq: f64,
    timer_start: f64,
}

impl Default for Platform {
    fn default() -> Self {
        pointer_init();
        Self {
            window: software_renderer::MinimalSoftwareWindow::new(
                software_renderer::RepaintBufferType::ReusedBuffer,
            ),
            timer_freq: timer_freq() as f64,
            timer_start: timer_tick() as f64,
        }
    }
}

impl slint::platform::Platform for Platform {
    fn create_window_adapter(
        &self,
    ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        Duration::from_secs_f64((timer_tick() as f64 - self.timer_start) / self.timer_freq)
    }

    fn run_event_loop(&self) -> Result<(), slint::PlatformError> {
        let gop_handle = uefi::boot::get_handle_for_protocol::<GraphicsOutput>().unwrap();
        let mut gop = uefi::boot::open_protocol_exclusive::<GraphicsOutput>(gop_handle).unwrap();

        // Query and print all available GOP modes
        info!("Querying available graphics resolutions:");
        let mut best_mode = None;
        let mut best_width = 0;
        let mut best_height = 0;

        for mode in gop.modes() {
            let info = mode.info();
            let (w, h) = info.resolution();
            info!("  - Mode: {}x{}", w, h);
            // Cap at 1920x1080 to prevent UEFI heap memory exhaustion (33MB for 4K is too large for UEFI allocator)
            // 1080p scales with a perfect 2:1 integer ratio on 4K displays, keeping text perfectly crisp
            if w <= 1920 && h <= 1080 {
                if w > best_width || (w == best_width && h > best_height) {
                    best_width = w;
                    best_height = h;
                    best_mode = Some(mode);
                }
            }
        }

        if let Some(ref mode) = best_mode {
            info!("Setting best graphics resolution: {}x{}", best_width, best_height);
            if let Err(e) = gop.set_mode(mode) {
                log::warn!("Failed to set graphics mode: {:?}", e);
            }
        }

        let info = gop.current_mode_info();
        let resolution = info.resolution();
        let screen_width = resolution.0;
        let screen_height = resolution.1;

        let mut fb = alloc::vec![SlintBltPixel(BltPixel::new(0, 0, 0)); screen_width * screen_height];
        let mut mfb = alloc::vec![BltPixel::new(0, 0, 0); POINTER_WIDTH * POINTER_HEIGHT];

        self.window.set_size(slint::PhysicalSize::new(
            screen_width.try_into().unwrap(),
            screen_height.try_into().unwrap(),
        ));

        let scale_factor = if screen_width >= 3840 {
            2.0f32
        } else if screen_width >= 2560 {
            1.5f32
        } else {
            1.0f32
        };
        let _ = self.window.try_dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor });

        let mut phys_x = (screen_width / 2) as f32;
        let mut phys_y = (screen_height / 2) as f32;
        let mut is_mouse_move = false;

        unsafe {
            GLOBAL_GOP = Some(&mut *gop as *mut GraphicsOutput);
            GLOBAL_FB = Some(fb.as_mut_slice() as *mut [SlintBltPixel]);
            GLOBAL_WINDOW = Some(Rc::downgrade(&self.window));
        }

        loop {
            let mut ip_str = None;
            let boot_selection = crate::poll_network_from_slint(&mut ip_str);
            
            if let Some(selection) = boot_selection {
                match selection {
                    crate::web::BootSelection::Windows => crate::boot::boot_os("\\EFI\\Microsoft\\Boot\\bootmgfw.efi"),
                    crate::web::BootSelection::Linux => crate::boot::boot_linux_direct(),
                }
            }

            if let Some(ip) = ip_str {
                crate::update_slint_ip(&ip);
            }

            slint::platform::update_timers_and_animations();

            // Keyboard input
            let mut direct_boot = None;
            while let Some(key) = get_key_press() {
                if key == '1' || key == 'w' || key == 'W' {
                    direct_boot = Some(crate::web::BootSelection::Windows);
                } else if key == '2' || key == 'l' || key == 'L' {
                    direct_boot = Some(crate::web::BootSelection::Linux);
                }

                let text = slint::SharedString::from(key);
                let _ = self.window.try_dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
                let _ = self.window.try_dispatch_event(WindowEvent::KeyReleased { text });
            }

            if let Some(selection) = direct_boot {
                match selection {
                    crate::web::BootSelection::Windows => {
                        info!("Direct keyboard boot: Windows");
                        crate::boot::boot_os("\\EFI\\Microsoft\\Boot\\bootmgfw.efi");
                    }
                    crate::web::BootSelection::Linux => {
                        info!("Direct keyboard boot: Linux");
                        crate::boot::boot_linux_direct();
                    }
                }
            }

            // Mouse input
            let pointers = unsafe { &mut *core::ptr::addr_of_mut!(MOUSE_POINTERS) };
            for mpointer in pointers.iter_mut() {
                loop {
                    match mpointer.read_state() {
                        Ok(Some(mouse)) => {
                            // Raw movement counts are typically small integers. 
                            // Add them directly with a sensitivity factor.
                            // We scale sensitivity by scale_factor so physical mouse movement translates naturally on high-DPI screens.
                            let sensitivity = 2.0 * scale_factor;
                            phys_x += (mouse.relative_movement[0] as f32) * sensitivity;
                            phys_y += (mouse.relative_movement[1] as f32) * sensitivity;

                            // Clamp physical position to physical screen bounds
                            phys_x = phys_x.clamp(0.0, (screen_width - POINTER_WIDTH) as f32);
                            phys_y = phys_y.clamp(0.0, (screen_height - POINTER_HEIGHT) as f32);

                            let button = match mouse.button {
                                [true, _] => PointerEventButton::Left,
                                [_, true] => PointerEventButton::Right,
                                _ => PointerEventButton::Other,
                            };

                            let logical_pos = slint::LogicalPosition::new(phys_x / scale_factor, phys_y / scale_factor);

                            let _ = self.window.try_dispatch_event(WindowEvent::PointerMoved { position: logical_pos });
                            let _ = self.window.try_dispatch_event(WindowEvent::PointerExited {});
                            if mouse.button[0] || mouse.button[1] {
                                let _ = self.window.try_dispatch_event(WindowEvent::PointerPressed { position: logical_pos, button });
                                  let _ = self.window.try_dispatch_event(WindowEvent::PointerReleased { position: logical_pos, button });
                            }
                            if mouse.relative_movement[2] != 0 {
                                let delta_y = -(mouse.relative_movement[2] as f32) * 30.0;
                                let _ = self.window.try_dispatch_event(WindowEvent::PointerScrolled {
                                    position: logical_pos,
                                    delta_x: 0.0,
                                    delta_y,
                                });
                            }
                            is_mouse_move = true;
                        }
                        Ok(None) => break,
                        Err(e) => {
                            log::warn!("Raw UEFI mouse error: {:?}", e);
                            break;
                        }
                    }
                }
            }

            if is_mouse_move {
                self.window.request_redraw();
                is_mouse_move = false;
            }

            self.window.draw_if_needed(|renderer| {
                renderer.render(&mut fb, screen_width);

                let blt_fb = unsafe { slice::from_raw_parts(fb.as_ptr() as *const BltPixel, fb.len()) };
                let blt_mfb = unsafe { slice::from_raw_parts_mut(mfb.as_mut_ptr() as *mut BltPixel, mfb.len()) };

                let _ = gop.blt(BltOp::BufferToVideo {
                    buffer: blt_fb,
                    src: BltRegion::Full,
                    dest: (0, 0),
                    dims: (screen_width, screen_height),
                });

                let _ = gop.blt(BltOp::VideoToBltBuffer {
                    buffer: blt_mfb,
                    src: (phys_x as usize, phys_y as usize),
                    dest: BltRegion::Full,
                    dims: (POINTER_WIDTH, POINTER_HEIGHT),
                });

                for y in 0..POINTER_HEIGHT {
                    for x in 0..POINTER_WIDTH {
                        let idx = x + y * POINTER_WIDTH;
                        let pixel_type = POINTER_PIXELS[idx];
                        if pixel_type == 1 {
                            blt_mfb[idx] = BltPixel::new(255, 255, 255);
                        } else if pixel_type == 2 {
                            blt_mfb[idx] = BltPixel::new(0, 0, 0);
                        }
                    }
                }

                let _ = gop.blt(BltOp::BufferToVideo {
                    buffer: blt_mfb,
                    src: BltRegion::Full,
                    dest: (phys_x as usize, phys_y as usize),
                    dims: (POINTER_WIDTH, POINTER_HEIGHT),
                });
            });

            if !self.window.has_active_animations() {
                wait_for_input(slint::platform::duration_until_next_timer_update());
            }
        }
    }
}

pub fn force_flush_logs() {
    unsafe {
        let in_flush_ptr = core::ptr::addr_of_mut!(IN_FLUSH);
        if *in_flush_ptr {
            return;
        }
        *in_flush_ptr = true;

        let window_ptr = core::ptr::addr_of!(GLOBAL_WINDOW);
        let window_weak = match &*window_ptr {
            Some(w) => w,
            None => {
                *in_flush_ptr = false;
                return;
            }
        };
        let window = match window_weak.upgrade() {
            Some(w) => w,
            None => {
                *in_flush_ptr = false;
                return;
            }
        };
        let gop_ptr = match GLOBAL_GOP {
            Some(p) => p,
            None => {
                *in_flush_ptr = false;
                return;
            }
        };
        let fb_ptr = match GLOBAL_FB {
            Some(p) => p,
            None => {
                *in_flush_ptr = false;
                return;
            }
        };

        window.request_redraw();

        let gop = &mut *gop_ptr;
        let fb = &mut *fb_ptr;
        let screen_width = gop.current_mode_info().resolution().0;
        let screen_height = gop.current_mode_info().resolution().1;

        window.draw_if_needed(|renderer| {
            renderer.render(fb, screen_width);
            let blt_fb = slice::from_raw_parts(fb.as_ptr() as *const BltPixel, fb.len());
            let _ = gop.blt(BltOp::BufferToVideo {
                buffer: blt_fb,
                src: BltRegion::Full,
                dest: (0, 0),
                dims: (screen_width, screen_height),
            });
        });

        *in_flush_ptr = false;
    }
}
