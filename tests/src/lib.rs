use std::path::{Path, PathBuf};
use std::process::{Command, Stdio, Child};
use std::sync::{Arc, Mutex, Once};
use std::thread;
use std::time::{Duration, Instant};
use std::fs;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU16, Ordering};
use std::io::{BufRead, BufReader, Write, Read};

static BUILD_ONCE: Once = Once::new();
static MONITOR_PORT_COUNTER: AtomicU16 = AtomicU16::new(29000);
static HTTP_PORT_COUNTER: AtomicU16 = AtomicU16::new(31000);

fn get_free_port(counter: &AtomicU16) -> u16 {
    loop {
        let port = counter.fetch_add(1, Ordering::SeqCst);
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
}

pub struct QemuResult {
    pub serial_output: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

pub struct QemuRunner {
    pub test_name: String,
    pub temp_dir: tempfile::TempDir,
    pub monitor_port: u16,
    pub http_port: u16,
    pub qemu_process: Option<Child>,
    pub serial_output: Arc<Mutex<String>>,
}

fn find_qemu_path() -> PathBuf {
    if let Ok(path) = std::env::var("QEMU_PATH") {
        return PathBuf::from(path);
    }
    if cfg!(windows) {
        PathBuf::from("C:\\Program Files\\qemu\\qemu-system-x86_64.exe")
    } else {
        PathBuf::from("qemu-system-x86_64")
    }
}

fn find_ovmf_paths() -> (PathBuf, PathBuf) {
    if let (Ok(code), Ok(vars)) = (std::env::var("OVMF_CODE_PATH"), std::env::var("OVMF_VARS_PATH")) {
        return (PathBuf::from(code), PathBuf::from(vars));
    }
    if cfg!(windows) {
        let code = PathBuf::from("C:\\Program Files\\qemu\\share\\edk2-x86_64-code.fd");
        let vars = PathBuf::from("C:\\Program Files\\qemu\\share\\edk2-i386-vars.fd");
        (code, vars)
    } else {
        // Linux common paths
        let code_candidates = [
            "/usr/share/OVMF/OVMF_CODE_4M.fd",
            "/usr/share/OVMF/OVMF_CODE.fd",
            "/usr/share/OVMF/OVMF_CODE.ms.fd",
            "/usr/share/ovmf/OVMF_CODE.fd",
            "/usr/share/OVMF/ovmf_code_x64.bin",
        ];
        let vars_candidates = [
            "/usr/share/OVMF/OVMF_VARS_4M.fd",
            "/usr/share/OVMF/OVMF_VARS.fd",
            "/usr/share/OVMF/OVMF_VARS.ms.fd",
            "/usr/share/ovmf/OVMF_VARS.fd",
            "/usr/share/OVMF/ovmf_vars_x64.bin",
        ];
        let code = code_candidates.iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
            .unwrap_or_else(|| PathBuf::from("/usr/share/OVMF/OVMF_CODE.fd"));
        let vars = vars_candidates.iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
            .unwrap_or_else(|| PathBuf::from("/usr/share/OVMF/OVMF_VARS.fd"));
        (code, vars)
    }
}

impl QemuRunner {
    pub fn new(test_name: &str) -> Self {
        // Pre-build target binaries once
        Self::build_all();

        let temp_dir = tempfile::Builder::new()
            .prefix(&format!("webuiboot_test_{}_", test_name))
            .tempdir()
            .expect("Failed to create temporary directory");

        let monitor_port = get_free_port(&MONITOR_PORT_COUNTER);
        let http_port = get_free_port(&HTTP_PORT_COUNTER);

        QemuRunner {
            test_name: test_name.to_string(),
            temp_dir,
            monitor_port,
            http_port,
            qemu_process: None,
            serial_output: Arc::new(Mutex::new(String::new())),
        }
    }

    fn build_all() {
        BUILD_ONCE.call_once(|| {
            println!("Building UEFI binaries...");
            let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
            let workspace_root = manifest_dir.parent().expect("workspace root");

            let cargo_exe = if cfg!(windows) {
                let user_profile = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\Ilia".to_string());
                let path = Path::new(&user_profile).join(".cargo").join("bin").join("cargo.exe");
                if path.exists() {
                    path.to_string_lossy().to_string()
                } else {
                    "cargo".to_string()
                }
            } else {
                "cargo".to_string()
            };

            // Build webuiboot
            let status = Command::new(&cargo_exe)
                .args(&[
                    "build",
                    "--release",
                    "-p",
                    "webuiboot",
                    "--target",
                    "x86_64-unknown-uefi",
                    "-Z",
                    "build-std=core,compiler_builtins,alloc",
                ])
                .current_dir(workspace_root)
                .status()
                .expect("Failed to run cargo build for webuiboot");
            assert!(status.success(), "Failed to compile webuiboot");

            // Build fake-bootloader
            let status = Command::new(&cargo_exe)
                .args(&[
                    "build",
                    "--release",
                    "-p",
                    "fake-bootloader",
                    "--target",
                    "x86_64-unknown-uefi",
                    "-Z",
                    "build-std=core,compiler_builtins,alloc",
                ])
                .current_dir(workspace_root)
                .status()
                .expect("Failed to run cargo build for fake-bootloader");
            assert!(status.success(), "Failed to compile fake-bootloader");
        });
    }

    pub fn prepare_esp(&self, bootnext: Option<&str>) {
        let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let workspace_root = manifest_dir.parent().unwrap();
        let target_dir = workspace_root.join("target").join("x86_64-unknown-uefi").join("release");

        let webuiboot_efi = target_dir.join("webuiboot.efi");
        let fake_windows_efi = target_dir.join("fake_windows.efi");
        let fake_linux_efi = target_dir.join("fake_linux.efi");

        assert!(webuiboot_efi.exists(), "webuiboot.efi does not exist at {:?}", webuiboot_efi);
        assert!(fake_windows_efi.exists(), "fake_windows.efi does not exist at {:?}", fake_windows_efi);
        assert!(fake_linux_efi.exists(), "fake_linux.efi does not exist at {:?}", fake_linux_efi);

        let esp_dir = self.temp_dir.path().join("esp");
        let boot_dir = esp_dir.join("EFI").join("BOOT");
        let ms_dir = esp_dir.join("EFI").join("Microsoft").join("Boot");
        let entries_dir = esp_dir.join("loader").join("entries");

        fs::create_dir_all(&boot_dir).unwrap();
        fs::create_dir_all(&ms_dir).unwrap();
        fs::create_dir_all(&entries_dir).unwrap();

        // Copy webuiboot as default EFI bootloader
        fs::copy(&webuiboot_efi, boot_dir.join("BOOTX64.EFI")).unwrap();

        // Copy fake-bootloader as Windows bootmgfw.efi
        fs::copy(&fake_windows_efi, ms_dir.join("bootmgfw.efi")).unwrap();

        // Copy fake-bootloader as pretend Linux kernel
        fs::copy(&fake_linux_efi, esp_dir.join("vmlinuz-test")).unwrap();

        // Create loader entries test.conf
        let conf_content = "title Test Linux\nlinux /vmlinuz-test\noptions quiet\n";
        fs::write(entries_dir.join("test.conf"), conf_content).unwrap();

        // Optional bootnext
        if let Some(content) = bootnext {
            fs::write(esp_dir.join("bootnext.txt"), content).unwrap();
            fs::write(esp_dir.join("EFI").join("bootnext.txt"), content).unwrap();
        }
    }

    pub fn start_qemu(&mut self) {
        let qemu_path = find_qemu_path();
        let (ovmf_code, ovmf_vars) = find_ovmf_paths();

        assert!(ovmf_code.exists(), "OVMF code file not found at {:?}", ovmf_code);
        assert!(ovmf_vars.exists(), "OVMF vars template not found at {:?}", ovmf_vars);

        // Copy vars file to temp dir to make it writable and avoid cross-test pollution
        let test_vars_path = self.temp_dir.path().join("ovmf-vars.fd");
        fs::copy(&ovmf_vars, &test_vars_path).unwrap();

        let esp_dir = self.temp_dir.path().join("esp");

        let mut qemu_args = vec![
            "-machine".to_string(), "q35".to_string(),
            "-m".to_string(), "256M".to_string(),
            "-drive".to_string(), format!("if=pflash,format=raw,readonly=on,file={}", ovmf_code.to_string_lossy()),
            "-drive".to_string(), format!("if=pflash,format=raw,file={}", test_vars_path.to_string_lossy()),
            "-drive".to_string(), format!("format=raw,file=fat:rw:{}", esp_dir.to_string_lossy()),
            "-nographic".to_string(),
            "-serial".to_string(), "stdio".to_string(),
            "-monitor".to_string(), format!("telnet:127.0.0.1:{},server,nowait", self.monitor_port),
            "-nic".to_string(), format!("user,model=e1000,net=10.0.2.0/24,dhcpstart=10.0.2.15,hostfwd=tcp:127.0.0.1:{}-:80", self.http_port),
            "-no-reboot".to_string(),
        ];

        // Virtualization acceleration support
        if cfg!(windows) {
            qemu_args.push("-accel".to_string());
            qemu_args.push("whpx".to_string());
            qemu_args.push("-accel".to_string());
            qemu_args.push("tcg".to_string());
        } else {
            qemu_args.push("-accel".to_string());
            qemu_args.push("kvm".to_string());
            qemu_args.push("-accel".to_string());
            qemu_args.push("tcg".to_string());
        }

        println!("[{}] Launching QEMU: {:?}", self.test_name, qemu_args);

        let mut child = Command::new(&qemu_path)
            .args(&qemu_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to start QEMU");

        let stdout = child.stdout.take().expect("Failed to take QEMU stdout");
        let serial_output = self.serial_output.clone();
        let test_name = self.test_name.clone();

        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(line) = line {
                    println!("[QEMU-{}] {}", test_name, line);
                    let mut output = serial_output.lock().unwrap();
                    output.push_str(&line);
                    output.push('\n');
                }
            }
        });

        self.qemu_process = Some(child);
    }

    pub fn send_key(&mut self, key: &str) -> std::io::Result<()> {
        println!("[{}] Connecting to telnet monitor on 127.0.0.1:{}...", self.test_name, self.monitor_port);
        let mut stream = std::net::TcpStream::connect(format!("127.0.0.1:{}", self.monitor_port))?;
        thread::sleep(Duration::from_millis(500));
        
        let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
        let mut greeting = [0u8; 4096];
        if let Ok(n) = stream.read(&mut greeting) {
            println!("[{}] Telnet monitor greeting: {:?}", self.test_name, String::from_utf8_lossy(&greeting[..n]));
        }

        let cmd = format!("sendkey {}\r\n", key);
        println!("[{}] Sending command: {:?}", self.test_name, cmd);
        stream.write_all(cmd.as_bytes())?;
        stream.flush()?;

        thread::sleep(Duration::from_millis(300));
        let mut response = [0u8; 4096];
        if let Ok(n) = stream.read(&mut response) {
            println!("[{}] Telnet monitor response: {:?}", self.test_name, String::from_utf8_lossy(&response[..n]));
        }

        Ok(())
    }

    pub fn send_http_boot(&self, os: &str) -> std::io::Result<()> {
        let mut stream = std::net::TcpStream::connect(format!("127.0.0.1:{}", self.http_port))?;
        let request = format!(
            "POST /boot/{} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            os, self.http_port
        );
        stream.write_all(request.as_bytes())?;
        stream.flush()?;
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        println!("[HTTP-response-{}] {}", self.test_name, response);
        Ok(())
    }

    pub fn wait_for_serial(&self, expected: &str, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            let output = self.serial_output.lock().unwrap();
            if output.contains(expected) {
                return true;
            }
            drop(output);
            thread::sleep(Duration::from_millis(100));
        }
        false
    }

    pub fn wait_and_collect(&mut self, timeout: Duration) -> QemuResult {
        let start = Instant::now();
        let mut timed_out = false;
        let mut exit_code = None;

        if let Some(mut child) = self.qemu_process.take() {
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        exit_code = status.code();
                        break;
                    }
                    Ok(None) => {
                        if start.elapsed() >= timeout {
                            println!("[{}] Timeout reached, killing QEMU process.", self.test_name);
                            let _ = child.kill();
                            let _ = child.wait();
                            timed_out = true;
                            break;
                        }
                        thread::sleep(Duration::from_millis(200));
                    }
                    Err(e) => {
                        println!("[{}] Error waiting for child process: {:?}", self.test_name, e);
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            }
        }

        let serial_output = self.serial_output.lock().unwrap().clone();
        QemuResult {
            serial_output,
            exit_code,
            timed_out,
        }
    }
}

impl Drop for QemuRunner {
    fn drop(&mut self) {
        if let Some(mut child) = self.qemu_process.take() {
            println!("[{}] Drop: killing running QEMU process", self.test_name);
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
