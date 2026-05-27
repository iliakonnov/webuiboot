use alloc::string::String;

pub const HTML_UI: &str = include_str!("../ui/index.html");

pub enum BootSelection {
    Windows,
    Linux,
}

pub struct Response<'a> {
    status_code: u16,
    status_text: &'a str,
    headers: alloc::vec::Vec<(alloc::string::String, alloc::string::String)>,
    body: &'a [u8],
}

impl<'a> Response<'a> {
    pub fn new(status_code: u16, status_text: &'a str) -> Self {
        Self {
            status_code,
            status_text,
            headers: alloc::vec::Vec::new(),
            body: &[],
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((alloc::string::String::from(name), alloc::string::String::from(value)));
        self
    }

    pub fn body(mut self, body: &'a [u8], content_type: &str) -> Self {
        self.body = body;
        self = self.header("Content-Type", content_type);
        self = self.header("Content-Length", &alloc::format!("{}", body.len()));
        self
    }

    pub fn serialize(&self) -> String {
        use core::fmt::Write;
        let mut response = String::new();
        let _ = write!(
            response,
            "HTTP/1.1 {} {}\r\n",
            self.status_code, self.status_text
        );
        for (name, val) in &self.headers {
            let _ = write!(response, "{}: {}\r\n", name, val);
        }
        let _ = write!(response, "\r\n");
        if !self.body.is_empty() {
            if let Ok(body_str) = core::str::from_utf8(self.body) {
                response.push_str(body_str);
            }
        }
        response
    }
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
        let response = Response::new(200, "OK")
            .header("Connection", "close")
            .body(HTML_UI.as_bytes(), "text/html")
            .serialize();
        Some((response, None))
    } else if method == "POST" && path == "/boot/windows" {
        let response = Response::new(200, "OK")
            .header("Connection", "close")
            .body(b"OK", "text/plain")
            .serialize();
        Some((response, Some(BootSelection::Windows)))
    } else if method == "POST" && path == "/boot/linux" {
        let response = Response::new(200, "OK")
            .header("Connection", "close")
            .body(b"OK", "text/plain")
            .serialize();
        Some((response, Some(BootSelection::Linux)))
    } else {
        let response = Response::new(404, "NOT FOUND")
            .header("Connection", "close")
            .serialize();
        Some((response, None))
    }
}
