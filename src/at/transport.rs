//! AT transport abstraction.
//!
//! Higher-level clients (BLE, Wi-Fi helpers, …) are generic over
//! [`AtTransport`] so the same code runs on either firmware architecture:
//!
//! * T01: [`ProcessorTransport`], an adapter over
//!   [`crate::at::processor::AtProcessor`] and the SPI bus.
//! * T02: [`crate::net::xarxa::Control`], which shares the SPI link with the
//!   raw-L2 data path.

use embassy_futures::select::{select, Either};
use embassy_time::Duration;
use embedded_hal::digital::OutputPin;
use embedded_hal_async::spi::SpiDevice;

use crate::at::command::AtCommandString;
use crate::at::parser::AtResponse;
use crate::at::processor::{AtProcessor, ResponseSlot};
use crate::at::reply::{AtOutput, AtStatus};
use crate::at::LineBuffer;
use crate::bus::SpiTransport;
use crate::error::{Error, Result};
use crate::sync::TmMutex;

/// Something that can execute one AT command and return its response.
///
/// Implementations are expected to be usable from several tasks
/// concurrently, serialising access to the underlying bus.
pub trait AtTransport {
    /// Send `command` (without a line ending) and wait for the terminal
    /// response.
    async fn at(&self, command: &str) -> Result<AtOutput>;

    /// Send a command whose payload **follows** the AT line.
    ///
    /// The reference driver terminates these commands with a trailing comma and
    /// then sends the bytes (GATT notifications, writes, `AT+CIPSEND`, …).
    /// Transports that cannot stream a payload return
    /// [`Error::NotSupported`], which is the default.
    async fn at_with_payload(&self, command: &str, payload: &[u8]) -> Result<AtOutput> {
        let _ = (command, payload);
        Err(Error::NotSupported)
    }
}

/// A shared transport is itself a transport.
///
/// This lets a driver hand out clients that **borrow** its transport, which is
/// what T02 needs: [`crate::net::xarxa::Control`] is stateful and cannot be
/// duplicated, so `&Control` is the transport.
impl<T: AtTransport + ?Sized> AtTransport for &T {
    async fn at(&self, command: &str) -> Result<AtOutput> {
        (**self).at(command).await
    }

    async fn at_with_payload(&self, command: &str, payload: &[u8]) -> Result<AtOutput> {
        (**self).at_with_payload(command, payload).await
    }
}

/// Convenience helpers built on [`AtTransport`], used by the typed clients.
pub trait AtTransportExt: AtTransport {
    /// Run a built command and return its response.
    async fn exec(&self, command: Result<AtCommandString>) -> Result<AtOutput> {
        let command = command?;
        self.at(crate::at::command::wire_str(&command)).await
    }

    /// Run a built command and require `OK`.
    async fn exec_ok(&self, command: Result<AtCommandString>) -> Result<()> {
        let out = self.exec(command).await?;
        if out.status == AtStatus::Ok {
            Ok(())
        } else {
            Err(Error::AtCommandFailed)
        }
    }

    /// Run a built query and return the value after `prefix`.
    async fn query(&self, command: Result<AtCommandString>, prefix: &str) -> Result<LineBuffer> {
        let out = self.exec(command).await?;
        let value = out.value_after(prefix).ok_or(Error::InvalidResponse)?;
        let mut line = LineBuffer::new();
        line.push_str(value).map_err(|_| Error::BufferTooSmall)?;
        Ok(line)
    }

    /// Run a built command with a payload and require `OK`.
    async fn exec_payload(&self, command: Result<AtCommandString>, payload: &[u8]) -> Result<()> {
        let command = command?;
        let out = self
            .at_with_payload(crate::at::command::wire_str(&command), payload)
            .await?;
        if out.status == AtStatus::Ok {
            Ok(())
        } else {
            Err(Error::AtCommandFailed)
        }
    }
}

impl<T: AtTransport + ?Sized> AtTransportExt for T {}

/// Complete an AT command with CRLF.
fn command_bytes(command: &str) -> Result<AtCommandString> {
    let mut bytes = AtCommandString::new();
    bytes
        .push_str(command)
        .map_err(|_| Error::BufferTooSmall)?;
    bytes.push_str("\r\n").map_err(|_| Error::BufferTooSmall)?;
    Ok(bytes)
}

/// Turn an information [`AtResponse`] into a text line, if it carries one.
///
/// `Data` lines are re-rendered as `prefix:content` so that callers can use
/// [`AtOutput::value_after`].
fn push_line(out: &mut AtOutput, response: AtResponse) {
    let mut line = LineBuffer::new();
    match response {
        AtResponse::Data { prefix, content } => {
            if line.push_str(prefix.as_str()).is_err()
                || line.push(':').is_err()
                || line.push_str(content.as_str()).is_err()
            {
                return;
            }
        }
        AtResponse::Raw(text) => {
            if line.push_str(text.as_str()).is_err() {
                return;
            }
        }
        _ => return,
    }
    let _ = out.lines.push(line);
}

/// T01 [`AtTransport`]: drives the shared [`AtProcessor`] task.
///
/// The processor owns the RX path; this adapter only submits commands and
/// collects the responses they produce.
pub struct ProcessorTransport<'a, SPI, CS>
where
    SPI: SpiDevice,
    CS: OutputPin,
{
    processor: &'a AtProcessor,
    spi: &'a TmMutex<SpiTransport<SPI, CS>>,
    timeout: Duration,
}

impl<'a, SPI, CS> ProcessorTransport<'a, SPI, CS>
where
    SPI: SpiDevice,
    CS: OutputPin,
{
    /// Create an adapter over `processor` and the SPI transport.
    pub const fn new(
        processor: &'a AtProcessor,
        spi: &'a TmMutex<SpiTransport<SPI, CS>>,
        timeout: Duration,
    ) -> Self {
        Self {
            processor,
            spi,
            timeout,
        }
    }
}

impl<SPI, CS> ProcessorTransport<'_, SPI, CS>
where
    SPI: SpiDevice,
    CS: OutputPin,
{
    /// Collect information lines until `OK`, `ERROR` or the timeout.
    async fn collect(&self, slot: &ResponseSlot) -> AtOutput {
        let mut out = AtOutput {
            status: AtStatus::Timeout,
            lines: heapless::Vec::new(),
        };
        loop {
            match select(slot.receive_data_response(), slot.wait(self.timeout)).await {
                Either::First(response) => push_line(&mut out, response),
                Either::Second(Ok(AtResponse::Ok)) => {
                    out.status = AtStatus::Ok;
                    break;
                }
                Either::Second(Ok(AtResponse::Error)) => {
                    out.status = AtStatus::Error;
                    break;
                }
                Either::Second(Ok(other)) => push_line(&mut out, other),
                Either::Second(Err(_)) => break,
            }
        }
        out
    }
}

impl<SPI, CS> AtTransport for ProcessorTransport<'_, SPI, CS>
where
    SPI: SpiDevice,
    CS: OutputPin,
{
    async fn at(&self, command: &str) -> Result<AtOutput> {
        let bytes = command_bytes(command)?;
        let (slot, index) = self
            .processor
            .send_multi_response_command(self.spi, bytes.as_bytes())
            .await?;
        let out = self.collect(slot).await;
        self.processor.release_multi_response_slot(index).await;
        Ok(out)
    }

    async fn at_with_payload(&self, command: &str, payload: &[u8]) -> Result<AtOutput> {
        let bytes = command_bytes(command)?;
        let (slot, index) = self
            .processor
            .send_multi_response_command(self.spi, bytes.as_bytes())
            .await?;

        let mut out = AtOutput {
            status: AtStatus::Timeout,
            lines: heapless::Vec::new(),
        };

        // Phase 1: wait for the '>' prompt (or a terminal response).
        let mut prompted = false;
        loop {
            match select(slot.receive_data_response(), slot.wait(self.timeout)).await {
                Either::First(response) => push_line(&mut out, response),
                Either::Second(Ok(AtResponse::ReadyPrompt)) => {
                    prompted = true;
                    break;
                }
                Either::Second(Ok(AtResponse::Ok)) => {
                    out.status = AtStatus::Ok;
                    break;
                }
                Either::Second(Ok(AtResponse::Error)) => {
                    out.status = AtStatus::Error;
                    break;
                }
                Either::Second(Ok(other)) => push_line(&mut out, other),
                Either::Second(Err(_)) => break,
            }
        }

        // Phase 2: send the payload and wait for the terminal response.
        if prompted {
            {
                let mut bus = self.spi.lock().await;
                bus.write(payload).await?;
            }
            loop {
                match select(slot.receive_data_response(), slot.wait(self.timeout)).await {
                    Either::First(response) => push_line(&mut out, response),
                    Either::Second(Ok(AtResponse::Ok)) => {
                        out.status = AtStatus::Ok;
                        break;
                    }
                    Either::Second(Ok(AtResponse::Error)) => {
                        out.status = AtStatus::Error;
                        break;
                    }
                    Either::Second(Ok(other)) => push_line(&mut out, other),
                    Either::Second(Err(_)) => break,
                }
            }
        }

        self.processor.release_multi_response_slot(index).await;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_bytes_appends_crlf() {
        assert_eq!(
            command_bytes("AT+BLENAME?").unwrap().as_str(),
            "AT+BLENAME?\r\n"
        );
    }

    #[test]
    fn data_lines_are_rendered_with_colon() {
        let mut out = AtOutput {
            status: AtStatus::Timeout,
            lines: heapless::Vec::new(),
        };
        let mut prefix = LineBuffer::new();
        prefix.push_str("+BLENAME").unwrap();
        let mut content = LineBuffer::new();
        content.push_str("sensor").unwrap();
        push_line(&mut out, AtResponse::Data { prefix, content });
        assert_eq!(out.lines[0].as_str(), "+BLENAME:sensor");
        assert_eq!(out.value_after("+BLENAME:"), Some("sensor"));
    }

    #[test]
    fn non_information_responses_are_ignored() {
        let mut out = AtOutput {
            status: AtStatus::Timeout,
            lines: heapless::Vec::new(),
        };
        push_line(&mut out, AtResponse::Ok);
        push_line(&mut out, AtResponse::ReadyPrompt);
        assert!(out.lines.is_empty());
    }
}

/// Test doubles for the typed-client tests.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use core::cell::{Cell, RefCell};
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};

    /// A single-threaded executor for tests that never actually wait.
    pub fn block_on<F: Future>(fut: F) -> F::Output {
        let mut fut = pin!(fut);
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    /// An `OK` response with no information lines.
    pub fn ok_output() -> AtOutput {
        AtOutput {
            status: AtStatus::Ok,
            lines: heapless::Vec::new(),
        }
    }

    /// An `OK` response carrying one information line.
    pub fn output_with(line: &str) -> AtOutput {
        let mut lines = heapless::Vec::new();
        let mut b = LineBuffer::new();
        b.push_str(line).unwrap();
        lines.push(b).unwrap();
        AtOutput {
            status: AtStatus::Ok,
            lines,
        }
    }

    /// Records the commands it is asked to run.
    #[derive(Default)]
    pub struct Mock {
        /// Response returned by [`AtTransport::at`]; `OK` when `None`.
        pub reply: Option<AtOutput>,
        /// Last command seen.
        pub last: RefCell<heapless::String<160>>,
        /// Length of the last payload.
        pub last_payload: Cell<usize>,
        /// Number of command-only calls.
        pub commands: Cell<usize>,
        /// Number of payload calls.
        pub payload_sends: Cell<usize>,
        /// Whether [`AtTransport::at_with_payload`] is supported.
        pub payload_supported: bool,
    }

    impl Mock {
        fn record(&self, command: &str) {
            let mut s = heapless::String::new();
            s.push_str(command).unwrap();
            *self.last.borrow_mut() = s;
        }

        /// The last command, for assertions.
        pub fn last(&self) -> heapless::String<160> {
            self.last.borrow().clone()
        }
    }

    impl AtTransport for Mock {
        async fn at(&self, command: &str) -> Result<AtOutput> {
            self.commands.set(self.commands.get() + 1);
            self.record(command);
            Ok(self.reply.clone().unwrap_or_else(ok_output))
        }

        async fn at_with_payload(&self, command: &str, payload: &[u8]) -> Result<AtOutput> {
            if !self.payload_supported {
                return Err(Error::NotSupported);
            }
            self.payload_sends.set(self.payload_sends.get() + 1);
            self.record(command);
            self.last_payload.set(payload.len());
            Ok(ok_output())
        }
    }
}
