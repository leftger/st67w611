//! HTTP client implementation

use embassy_time::Duration;
use heapless::{String, Vec};

use crate::at::processor::AtProcessor;
use crate::bus::SpiTransport;
use crate::error::{Error, Result};
use crate::net::device::NetworkDevice;
use crate::sync::TmMutex;
use crate::types::{SocketId, SocketProtocol};

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
    Get,
    Post,
    Put,
    Delete,
    Head,
    Options,
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
    pub name: String<64>,
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
        value_buf.push_str(value).map_err(|_| Error::BufferTooSmall)?;

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
        body_buf.extend_from_slice(body).map_err(|_| Error::BufferTooSmall)?;
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
    /// AT processor
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
        _spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        _request: &HttpRequest,
    ) -> Result<HttpResponse>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        // TODO: Implement HTTP request/response handling
        // This would involve:
        // 1. Parse URL to get host, port, path
        // 2. Allocate a socket
        // 3. Connect to host
        // 4. Format and send HTTP request
        // 5. Receive and parse HTTP response
        // 6. Close socket
        // 7. Return parsed response

        Err(Error::NotSupported)
    }

    /// Perform a GET request
    pub async fn get<SPI, CS>(
        &self,
        spi: &'static TmMutex<SpiTransport<SPI, CS>>,
        url: &str,
    ) -> Result<HttpResponse>
    where
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
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
        SPI: embedded_hal_async::spi::SpiDevice,
        CS: embedded_hal::digital::OutputPin,
    {
        let request = HttpRequest::new(HttpMethod::Post, url)?
            .with_body(body)?
            .with_header("Content-Type", "application/json")?;
        self.request(spi, &request).await
    }
}
