use alloc::string::String;

pub const HTML_UI: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>UEFI Boot Selection</title>
    <style>
        :root {
            --bg-color: #0f172a;
            --text-color: #f8fafc;
            --btn-windows: #0284c7;
            --btn-linux: #ea580c;
        }
        body {
            background-color: var(--bg-color);
            color: var(--text-color);
            font-family: 'Segoe UI', Roboto, Helvetica, Arial, sans-serif;
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            height: 100vh;
            margin: 0;
            overflow: hidden;
        }
        h1 {
            font-size: 3rem;
            margin-bottom: 2rem;
            font-weight: 300;
            letter-spacing: 2px;
            animation: fadeIn 1s ease-in-out;
        }
        .btn-container {
            display: flex;
            gap: 2rem;
            animation: slideUp 1s ease-in-out;
        }
        button {
            border: none;
            border-radius: 12px;
            padding: 1.5rem 3rem;
            font-size: 1.5rem;
            font-weight: 600;
            color: white;
            cursor: pointer;
            transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
            box-shadow: 0 10px 15px -3px rgba(0, 0, 0, 0.5);
            width: 250px;
        }
        .btn-windows {
            background: linear-gradient(135deg, #0284c7, #0369a1);
        }
        .btn-linux {
            background: linear-gradient(135deg, #ea580c, #c2410c);
        }
        button:hover {
            transform: translateY(-5px) scale(1.05);
            box-shadow: 0 20px 25px -5px rgba(0, 0, 0, 0.5);
        }
        button:active {
            transform: translateY(0) scale(0.98);
        }
        .loading {
            opacity: 0.5;
            pointer-events: none;
        }
        @keyframes fadeIn {
            from { opacity: 0; }
            to { opacity: 1; }
        }
        @keyframes slideUp {
            from { opacity: 0; transform: translateY(30px); }
            to { opacity: 1; transform: translateY(0); }
        }
        @media (max-width: 640px) {
            h1 {
                font-size: 2rem;
                margin-bottom: 1.5rem;
                text-align: center;
                padding: 0 1rem;
            }
            .btn-container {
                flex-direction: column;
                gap: 1.2rem;
                width: 100%;
                align-items: center;
            }
            button {
                width: 80%;
                max-width: 280px;
                padding: 1.2rem 2rem;
                font-size: 1.3rem;
            }
        }
    </style>
</head>
<body>
    <h1>Select OS to Boot</h1>
    <div class="btn-container">
        <button class="btn-windows" onclick="boot('windows')">Windows</button>
        <button class="btn-linux" onclick="boot('linux')">Linux</button>
    </div>

    <script>
        async function boot(os) {
            const btns = document.querySelectorAll('button');
            btns.forEach(b => b.classList.add('loading'));
            try {
                await fetch('/boot/' + os, { method: 'POST' });
                document.body.innerHTML = '<h1 style="color: #4ade80;">Booting ' + os + '...</h1>';
            } catch(e) {
                document.body.innerHTML = '<h1 style="color: #f87171;">Failed to communicate with bootloader.</h1>';
            }
        }
    </script>
</body>
</html>"#;

pub enum BootSelection {
    Windows,
    Linux,
}

pub fn handle_http_request(request: &[u8]) -> Option<(String, Option<BootSelection>)> {
    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut req = httparse::Request::new(&mut headers);

    match req.parse(request) {
        Ok(_) => {}
        Err(_) => return None,
    }

    let path = req.path?;
    let method = req.method?;

    if method == "GET" && (path == "/" || path == "/index.html") {
        let response = alloc::format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            HTML_UI.len(),
            HTML_UI
        );
        Some((response, None))
    } else if method == "POST" && path == "/boot/windows" {
        let response = alloc::format!("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nOK");
        Some((response, Some(BootSelection::Windows)))
    } else if method == "POST" && path == "/boot/linux" {
        let response = alloc::format!("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nOK");
        Some((response, Some(BootSelection::Linux)))
    } else {
        let response = alloc::format!("HTTP/1.1 404 NOT FOUND\r\nConnection: close\r\n\r\n");
        Some((response, None))
    }
}
