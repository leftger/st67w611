//! HTTP client implementation

use core::fmt::Write as _;
use embassy_time::Duration;
use heapless::{String, Vec};

use crate::at::processor::AtProcessor;
use crate::bus::SpiTransport;
use crate::error::{Error, Result};
use crate::net::device::NetworkDevice;
use crate::sync::TmMutex;
use crate::types::SocketProtocol;

/// Parsed URL components
#[derive(Debug, Clone)]
pub struct ParsedUrl {
    /// Scheme (http or https)
    pub scheme: UrlScheme,
    /// Host/domain name
    pub host: String<128>,
    /// Port number
    pub port: u16,
    /// Path (including query string)
    pub path: String<128>,
}

/// URL scheme
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlScheme {
    /// HTTP (unsecured)
    Http,
    /// HTTPS (TLS-secured)
    Https,
}

impl UrlScheme {
    /// Get default port for this scheme
    pub const fn default_port(&self) -> u16 {
        match self {
            UrlScheme::Http => 80,
            UrlScheme::Https => 443,
        }
    }

    /// Get protocol type for socket
    pub const fn socket_protocol(&self) -> SocketProtocol {
        match self {
            UrlScheme::Http => SocketProtocol::Tcp,
            UrlScheme::Https => SocketProtocol::Ssl,
        }
    }
}

/// Parse a URL into components
pub fn parse_url(url: &str) -> Result<ParsedUrl> {
    let url = url.trim();

    // Parse scheme
    let (scheme, rest) = if url.starts_with("https://") {
        (UrlScheme::Https, &url[8..])
    } else if url.starts_with("http://") {
        (UrlScheme::Http, &url[7..])
    } else {
        // Assume HTTP if no scheme
        (UrlScheme::Http, url)
    };

    // Find the first slash to separate host from path
    let (host_port, path_str) = if let Some(slash_pos) = rest.find('/') {
        (&rest[..slash_pos], &rest[slash_pos..])
    } else {
        (rest, "/")
    };

    // Parse host and port
    let (host_str, port) = if let Some(colon_pos) = host_port.find(':') {
        let port_str = &host_port[colon_pos + 1..];
        let port = port_str
            .parse::<u16>()
            .map_err(|_| Error::InvalidParameter)?;
        (&host_port[..colon_pos], port)
    } else {
        (host_port, scheme.default_port())
    };

    // Build result
    let mut host = String::new();
    host.push_str(host_str).map_err(|_| Error::BufferTooSmall)?;

    let mut path = String::new();
    path.push_str(path_str).map_err(|_| Error::BufferTooSmall)?;

    Ok(ParsedUrl {
        scheme,
        host,
        port,
        path,
    })
}

/// Maximum URL length
pub const MAX_URL_LEN: usize = 256;

/// Maximum header count
pub const MAX_HEADERS: usize = 8;

/// Maximum header length
pub const MAX_HEADER_LEN: usize = 128;

/// HTTP method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum HttpMethod {
    /// HTTP GET method
    Get,
    /// HTTP POST method
    Post,
    /// HTTP PUT method
    Put,
    /// HTTP DELETE method
    Delete,
    /// HTTP HEAD method
    Head,
    /// HTTP OPTIONS method
    Options,
    /// HTTP PATCH method
    Patch,
}

impl HttpMethod {
    /// Get method as string
    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Head => "HEAD",
            HttpMethod::Options => "OPTIONS",
            HttpMethod::Patch => "PATCH",
        }
    }
}

/// HTTP header
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct HttpHeader {
    /// Header name (e.g., "Content-Type")
    pub name: String<64>,
    /// Header value (e.g., "application/json")
    pub value: String<64>,
}

/// HTTP request
#[derive(Debug, Clone)]
pub struct HttpRequest {
    /// HTTP method
    pub method: HttpMethod,
    /// URL
    pub url: String<MAX_URL_LEN>,
    /// Headers
    pub headers: Vec<HttpHeader, MAX_HEADERS>,
    /// Body (optional)
    pub body: Option<Vec<u8, 1024>>,
}

impl HttpRequest {
    /// Create a new HTTP request
    pub fn new(method: HttpMethod, url: &str) -> Result<Self> {
        let mut url_buf = String::new();
        url_buf.push_str(url).map_err(|_| Error::BufferTooSmall)?;

        Ok(Self {
            method,
            url: url_buf,
            headers: Vec::new(),
            body: None,
        })
    }

    /// Add a header
    pub fn with_header(mut self, name: &str, value: &str) -> Result<Self> {
        let mut name_buf = String::new();
        name_buf.push_str(name).map_err(|_| Error::BufferTooSmall)?;

        let mut value_buf = String::new();
        value_buf
            .push_str(value)
            .map_err(|_| Error::BufferTooSmall)?;

        self.headers
            .push(HttpHeader {
                name: name_buf,
                value: value_buf,
            })
            .map_err(|_| Error::BufferTooSmall)?;

        Ok(self)
    }

    /// Set the request body
    pub fn with_body(mut self, body: &[u8]) -> Result<Self> {
        let mut body_buf = Vec::new();
        body_buf
            .extend_from_slice(body)
            .map_err(|_| Error::BufferTooSmall)?;
        self.body = Some(body_buf);
        Ok(self)
    }
}

/// HTTP response
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct HttpResponse {
    /// Status code
    pub status_code: u16,
    /// Headers
    pub headers: Vec<HttpHeader, MAX_HEADERS>,
    /// Body
    pub body: Vec<u8, 2048>,
}

/// HTTP client
pub struct HttpClient {
    /// Network device
    device: &'static NetworkDevice,
    /// AT processor (used for future direct AT command support)
    #[allow(dead_code)]
    processor: &'static AtProcessor,
    /// Command timeout
    timeout: Duration,
}

impl HttpClient {
    /// Create a new HTTP client
    pub const fn new(
        device: &'static NetworkDevice,
        processor: &'static AtProcessor,
        timeout: Duration,
    ) -> Self {
        Self {
            device,
            processor,
            timeout,
        }
    }

    /// Send an HTTP request
    pub async fn request<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        request: &HttpRequest,
    ) -> Result<HttpResponse>
    where
        SPI: embedded_hal_async::spi::SpiDevice + 'static,
        CS: embedded_hal::digital::OutputPin + 'static,
    {
        // Parse URL
        let parsed = parse_url(&request.url)?;

        // Connect through the unified socket layer. `socket_protocol()` is TCP
        // for `http://` and SSL for `https://` (the module terminates TLS).
        let mut socket = crate::stack::TcpSocket::connect_with_protocol(
            self.device,
            spi,
            self.timeout,
            parsed.scheme.socket_protocol(),
            &parsed.host,
            parsed.port,
        )
        .await
        .map_err(|e| e.0)?;

        // Format HTTP request
        let mut request_str = String::<1024>::new();

        // Request line: METHOD /path HTTP/1.1
        write!(
            &mut request_str,
            "{} {} HTTP/1.1\r\n",
            request.method.as_str(),
            parsed.path.as_str()
        )
        .map_err(|_| Error::BufferTooSmall)?;

        // Host header (required for HTTP/1.1)
        write!(&mut request_str, "Host: {}\r\n", parsed.host.as_str())
            .map_err(|_| Error::BufferTooSmall)?;

        // User headers
        for header in &request.headers {
            write!(
                &mut request_str,
                "{}: {}\r\n",
                header.name.as_str(),
                header.value.as_str()
            )
            .map_err(|_| Error::BufferTooSmall)?;
        }

        // Content-Length if there's a body
        if let Some(ref body) = request.body {
            write!(&mut request_str, "Content-Length: {}\r\n", body.len())
                .map_err(|_| Error::BufferTooSmall)?;
        }

        // End of headers
        request_str
            .push_str("\r\n")
            .map_err(|_| Error::BufferTooSmall)?;

        // Send headers
        if embedded_io_async::Write::write_all(&mut socket, request_str.as_bytes())
            .await
            .is_err()
        {
            let _ = socket.close().await;
            return Err(Error::SocketError);
        }

        // Send body if present
        if let Some(ref body) = request.body {
            if embedded_io_async::Write::write_all(&mut socket, body)
                .await
                .is_err()
            {
                let _ = socket.close().await;
                return Err(Error::SocketError);
            }
        }

        // Receive and parse response
        let mut response_buffer = [0u8; 2048];
        let mut total_received = 0;

        // Read response with timeout
        let response_timeout = embassy_time::Instant::now() + self.timeout;

        while embassy_time::Instant::now() < response_timeout
            && total_received < response_buffer.len()
        {
            match embedded_io_async::Read::read(
                &mut socket,
                &mut response_buffer[total_received..],
            )
            .await
            {
                Ok(n) if n > 0 => {
                    total_received += n;
                    // For now, we'll assume we got the full response after one
                    // read. A proper implementation would check for
                    // Content-Length or chunked encoding.
                    break;
                }
                Ok(_) => break,
                Err(_) => break,
            }
        }

        // Close the socket
        let _ = socket.close().await;

        // Parse HTTP response
        self.parse_response(&response_buffer[..total_received])
    }

    /// Parse HTTP response
    fn parse_response(&self, data: &[u8]) -> Result<HttpResponse> {
        // Find the end of headers (\r\n\r\n)
        let mut header_end = 0;
        for i in 0..data.len().saturating_sub(3) {
            if &data[i..i + 4] == b"\r\n\r\n" {
                header_end = i + 4;
                break;
            }
        }

        if header_end == 0 {
            return Err(Error::InvalidResponse);
        }

        // Parse status line and headers
        let header_str =
            core::str::from_utf8(&data[..header_end]).map_err(|_| Error::InvalidResponse)?;

        let mut lines = header_str.lines();

        // Parse status line (e.g., "HTTP/1.1 200 OK")
        let status_line = lines.next().ok_or(Error::InvalidResponse)?;
        let mut status_parts = status_line.split_whitespace();
        status_parts.next(); // Skip HTTP version
        let status_code_str = status_parts.next().ok_or(Error::InvalidResponse)?;
        let status_code = status_code_str
            .parse::<u16>()
            .map_err(|_| Error::InvalidResponse)?;

        // Parse headers
        let mut headers = Vec::new();
        for line in lines {
            if line.is_empty() {
                break;
            }

            if let Some(colon_pos) = line.find(':') {
                let name_str = line[..colon_pos].trim();
                let value_str = line[colon_pos + 1..].trim();

                let mut name = String::new();
                name.push_str(name_str).map_err(|_| Error::BufferTooSmall)?;

                let mut value = String::new();
                value
                    .push_str(value_str)
                    .map_err(|_| Error::BufferTooSmall)?;

                if headers.push(HttpHeader { name, value }).is_err() {
                    // Too many headers, ignore rest
                    break;
                }
            }
        }

        // Copy body
        let mut body = Vec::new();
        for &byte in &data[header_end..] {
            if body.push(byte).is_err() {
                // Body too large
                break;
            }
        }

        Ok(HttpResponse {
            status_code,
            headers,
            body,
        })
    }

    /// Perform a GET request
    pub async fn get<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        url: &str,
    ) -> Result<HttpResponse>
    where
        SPI: embedded_hal_async::spi::SpiDevice + 'static,
        CS: embedded_hal::digital::OutputPin + 'static,
    {
        let request = HttpRequest::new(HttpMethod::Get, url)?;
        self.request(spi, &request).await
    }

    /// Perform a POST request
    pub async fn post<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        url: &str,
        body: &[u8],
    ) -> Result<HttpResponse>
    where
        SPI: embedded_hal_async::spi::SpiDevice + 'static,
        CS: embedded_hal::digital::OutputPin + 'static,
    {
        let request = HttpRequest::new(HttpMethod::Post, url)?
            .with_body(body)?
            .with_header("Content-Type", "application/json")?;
        self.request(spi, &request).await
    }

    /// Perform a PUT request
    pub async fn put<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        url: &str,
        body: &[u8],
    ) -> Result<HttpResponse>
    where
        SPI: embedded_hal_async::spi::SpiDevice + 'static,
        CS: embedded_hal::digital::OutputPin + 'static,
    {
        let request = HttpRequest::new(HttpMethod::Put, url)?
            .with_body(body)?
            .with_header("Content-Type", "application/json")?;
        self.request(spi, &request).await
    }

    /// Perform a HEAD request
    pub async fn head<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        url: &str,
    ) -> Result<HttpResponse>
    where
        SPI: embedded_hal_async::spi::SpiDevice + 'static,
        CS: embedded_hal::digital::OutputPin + 'static,
    {
        let request = HttpRequest::new(HttpMethod::Head, url)?;
        self.request(spi, &request).await
    }

    /// Perform a DELETE request
    pub async fn delete<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        url: &str,
    ) -> Result<HttpResponse>
    where
        SPI: embedded_hal_async::spi::SpiDevice + 'static,
        CS: embedded_hal::digital::OutputPin + 'static,
    {
        let request = HttpRequest::new(HttpMethod::Delete, url)?;
        self.request(spi, &request).await
    }
}
