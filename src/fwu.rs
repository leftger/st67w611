//! Firmware update (FOTA) over the AT layer.
//!
//! The sequence, from `Driver/W61_at/w61_at_sys.c`, is:
//!
//! 1. `AT+OTASTART=1` starts a transfer session,
//! 2. for each chunk, `AT+OTASEND=<len>` is answered with `>` and the chunk
//!    bytes are then sent, followed by `OK`,
//! 3. `AT+OTAFIN` finishes the transfer and reboots the module.
//!
//! The typed driver is generic over [`AtTransport`], so it runs on both
//! firmwares. It needs a transport that implements
//! [`AtTransport::at_with_payload`]; both in-tree transports do.
//!
//! ```no_run
//! use st67w611::fwu::Fwu;
//! use st67w611::at::AtTransport;
//!
//! async fn flash<C: AtTransport>(control: C, image: &[u8]) -> Result<(), st67w611::Error> {
//!     let fwu = Fwu::new(control);
//!     fwu.update(image, 1024).await
//! }
//! ```

use crate::at::command::{fwu as cmd, wire_str, AtCommandString};
use crate::at::reply::AtStatus;
use crate::at::transport::AtTransport;
use crate::bus::frame::MAX_PAYLOAD;
use crate::error::{Error, Result};

/// Largest chunk that fits one SPI frame.
pub const MAX_CHUNK: usize = MAX_PAYLOAD;

/// Firmware-update client.
///
/// `T` is any [`AtTransport`]; see the module docs.
pub struct Fwu<T> {
    transport: T,
}

impl<T> Fwu<T> {
    /// Create a firmware-update client over `transport`.
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    /// The underlying transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Consume the client, returning the transport.
    pub fn into_transport(self) -> T {
        self.transport
    }
}

impl<T: AtTransport> Fwu<T> {
    /// Run a command-only step and require `OK`.
    async fn exec(&self, command: Result<AtCommandString>) -> Result<()> {
        let command = command?;
        let out = self.transport.at(wire_str(&command)).await?;
        if out.status == AtStatus::Ok {
            Ok(())
        } else {
            Err(Error::AtCommandFailed)
        }
    }

    /// Start a transfer session (`AT+OTASTART=1`).
    pub async fn start(&self) -> Result<()> {
        self.exec(cmd::start(true)).await
    }

    /// Abort a transfer session (`AT+OTASTART=0`).
    pub async fn abort(&self) -> Result<()> {
        self.exec(cmd::start(false)).await
    }

    /// Send one chunk (`AT+OTASEND=<len>` then the bytes).
    pub async fn send(&self, chunk: &[u8]) -> Result<()> {
        if chunk.is_empty() || chunk.len() > MAX_CHUNK {
            return Err(Error::InvalidParameter);
        }
        let command = cmd::send(chunk.len() as u32)?;
        let out = self
            .transport
            .at_with_payload(wire_str(&command), chunk)
            .await?;
        if out.status == AtStatus::Ok {
            Ok(())
        } else {
            Err(Error::AtCommandFailed)
        }
    }

    /// Finish the transfer and reboot (`AT+OTAFIN`).
    pub async fn finish(&self) -> Result<()> {
        self.exec(cmd::finish()).await
    }

    /// Stream a whole image: start, all chunks, finish.
    ///
    /// `chunk_size` must be in `1..=MAX_CHUNK`. On any error the session is
    /// left as-is; call [`abort`](Self::abort) to cancel explicitly.
    pub async fn update(&self, image: &[u8], chunk_size: usize) -> Result<()> {
        if chunk_size == 0 || chunk_size > MAX_CHUNK {
            return Err(Error::InvalidParameter);
        }
        self.start().await?;
        for chunk in image.chunks(chunk_size) {
            self.send(chunk).await?;
        }
        self.finish().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::at::reply::AtOutput;
    use core::cell::{Cell, RefCell};
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};

    fn block_on<F: Future>(fut: F) -> F::Output {
        let mut fut = pin!(fut);
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    fn ok_output() -> AtOutput {
        AtOutput {
            status: AtStatus::Ok,
            lines: heapless::Vec::new(),
        }
    }

    #[derive(Default)]
    struct Mock {
        last: RefCell<heapless::String<64>>,
        last_payload: Cell<usize>,
        payload_sends: Cell<usize>,
        commands: Cell<usize>,
    }

    impl AtTransport for Mock {
        async fn at(&self, command: &str) -> Result<AtOutput> {
            self.commands.set(self.commands.get() + 1);
            let mut s = heapless::String::new();
            s.push_str(command).unwrap();
            *self.last.borrow_mut() = s;
            Ok(ok_output())
        }

        async fn at_with_payload(&self, command: &str, payload: &[u8]) -> Result<AtOutput> {
            self.payload_sends.set(self.payload_sends.get() + 1);
            let mut s = heapless::String::new();
            s.push_str(command).unwrap();
            *self.last.borrow_mut() = s;
            self.last_payload.set(payload.len());
            Ok(ok_output())
        }
    }

    #[test]
    fn start_and_finish() {
        let fwu = Fwu::new(Mock::default());
        block_on(fwu.start()).unwrap();
        assert_eq!(fwu.transport().last.borrow().as_str(), "AT+OTASTART=1");
        block_on(fwu.abort()).unwrap();
        assert_eq!(fwu.transport().last.borrow().as_str(), "AT+OTASTART=0");
        block_on(fwu.finish()).unwrap();
        assert_eq!(fwu.transport().last.borrow().as_str(), "AT+OTAFIN");
    }

    #[test]
    fn send_streams_chunk_with_length() {
        let fwu = Fwu::new(Mock::default());
        block_on(fwu.send(&[0u8; 512])).unwrap();
        assert_eq!(fwu.transport().last.borrow().as_str(), "AT+OTASEND=512");
        assert_eq!(fwu.transport().last_payload.get(), 512);
    }

    #[test]
    fn send_rejects_oversized_or_empty_chunks() {
        let fwu = Fwu::new(Mock::default());
        assert_eq!(block_on(fwu.send(&[])), Err(Error::InvalidParameter));
        let too_big = [0u8; MAX_CHUNK + 1];
        assert_eq!(block_on(fwu.send(&too_big)), Err(Error::InvalidParameter));
    }

    #[test]
    fn update_streams_whole_image() {
        let fwu = Fwu::new(Mock::default());
        let image = [0u8; 40];
        block_on(fwu.update(&image, 16)).unwrap();
        // start + finish are command-only; 40 bytes in 16-byte chunks = 3.
        assert_eq!(fwu.transport().payload_sends.get(), 3);
        assert_eq!(fwu.transport().commands.get(), 2);
    }

    #[test]
    fn update_rejects_bad_chunk_size() {
        let fwu = Fwu::new(Mock::default());
        assert_eq!(block_on(fwu.update(&[0u8; 4], 0)), Err(Error::InvalidParameter));
        assert_eq!(
            block_on(fwu.update(&[0u8; 4], MAX_CHUNK + 1)),
            Err(Error::InvalidParameter)
        );
    }
}
