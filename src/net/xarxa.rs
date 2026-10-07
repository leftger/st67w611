//! T02 raw-L2 driver: the ST67W611 as a WiFi MAC/PHY for `embassy-net`.
//!
//! In T02 firmware the TCP/IP stack runs on the host. The module forwards raw
//! Ethernet frames over the SPI link tagged as [`TrafficType::NetworkSta`] or
//! [`TrafficType::NetworkAp`], while AT traffic keeps using
//! [`TrafficType::AtCommand`] on the same bus.
//!
//! This module connects the module to `embassy-net`, which (as of the xarxa
//! rewrite) gives you IPv4, IPv6, TCP, UDP, DHCP, DNS/mDNS, SLAAC and ICMP for
//! free. Wi-Fi association still happens with AT commands through
//! [`Control::at`].
//!
//! # Overview
//!
//! ```text
//!   embassy-net::Stack (xarxa)
//!          ▲  │
//!   PacketBuf │ PacketBuf            ← embassy-net-driver-channel
//!          │  ▼
//!      Runner (this module)          ← owns the SPI Engine
//!          │  ▲
//!     Outbound  Received             ← bus::engine
//!          ▼  │
//!        ST67W611 (T02)
//! ```
//!
//! # Usage
//!
//! ```no_run
//! use st67w611::net::xarxa::{self, State};
//! # use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex as Cs;
//! # use embassy_sync::signal::Signal;
//! # use core::sync::atomic::AtomicBool;
//! # static READY: Signal<Cs, ()> = Signal::new();
//! # static HDR_ACK: Signal<Cs, ()> = Signal::new();
//! # static RDY_LEVEL: AtomicBool = AtomicBool::new(false);
//! # async fn f(spi: impl embedded_hal_async::spi::SpiBus, cs: impl embedded_hal::digital::OutputPin) {
//! static STATE: static_cell::StaticCell<State<4, 4>> = static_cell::StaticCell::new();
//! let (device, runner, control) = xarxa::new(
//!     spi, cs, &READY, &HDR_ACK, &RDY_LEVEL, STATE.init(State::new()), [0x02, 0, 0, 0, 0, 0x01],
//! );
//! # let _ = (device, runner, control);
//! # }
//! ```
//!
//! Then add `device` to a stack with
//! `embassy_net::Stack::add_iface_borrowed(&mut device)` and spawn
//! [`Runner::run`] in the background.

use core::sync::atomic::AtomicBool;

use embassy_futures::select::{select3, Either3};
use embassy_net_driver_channel as ch;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{with_timeout, Duration, Instant, Timer};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::spi::SpiBus;

use crate::at::parser::{self, AtResponse, LineBuffer};
use crate::bus::engine::{Engine, Outbound, Received};
use crate::bus::frame::{TrafficType, HEADER_LEN, MAX_PAYLOAD};
use crate::error::{Error, Result};

pub use ch::driver::{HardwareAddress, LinkState};
pub use ch::Device as WifiDevice;

/// An owned network packet, from the global xarxa pool.
pub use ch::driver::PacketBuf;

/// Ethernet MTU forwarded by the module.
pub const MTU: usize = 1514;

/// Longest AT command accepted by [`Control::at`].
pub const AT_CMD_MAX: usize = 256;

/// Joining an access point takes far longer than an ordinary command, so
/// `AT+CWJAP` gets its own budget (ST's driver uses `W61_WIFI_TIMEOUT`).
pub const WIFI_JOIN_TIMEOUT: Duration = Duration::from_secs(30);

/// A scan takes several seconds, so `AT+CWLAP` gets its own budget too.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(20);

/// The control side waits this much longer than the runner transaction budget,
/// so the runner always gets to report its verdict first. If both sides expired
/// together the caller would see a bare `Err(Timeout)` while the runner was
/// still busy with that command, and the next command would then fail `Busy`.
const RESPONSE_GRACE: Duration = Duration::from_secs(2);

/// Data lines collected per AT transaction (re-exported from [`crate::at`]).
pub use crate::at::reply::AT_LINES_MAX;

/// Default timeout for a complete AT transaction.
pub const AT_TIMEOUT: Duration = Duration::from_secs(5);

/// Staging buffer size: one maximum-sized frame including its header.
const RX_SCRATCH: usize = HEADER_LEN + MAX_PAYLOAD;

/// Largest payload that may follow an AT command (one SPI frame).
pub const AT_PAYLOAD_MAX: usize = MAX_PAYLOAD;

/// An AT transaction request.
///
/// Most commands have no payload; commands like `AT+OTASEND` or
/// `AT+BLEGATTSNTFY` carry one, sent after the module answers with `>`.
pub struct AtRequest {
    /// The command line, CRLF included.
    command: heapless::Vec<u8, AT_CMD_MAX>,
    /// Optional payload sent after the module's `>` prompt.
    payload: heapless::Vec<u8, AT_PAYLOAD_MAX>,
    /// How long the runner waits for this command's terminal response.
    ///
    /// Per-request rather than global: joining an AP legitimately takes tens of
    /// seconds, while an ordinary command should fail fast.
    timeout: Duration,
}

impl AtRequest {
    /// A plain command; CRLF is appended.
    fn command(cmd: &str, timeout: Duration) -> Result<Self> {
        let mut command = heapless::Vec::new();
        command
            .extend_from_slice(cmd.as_bytes())
            .map_err(|_| Error::BufferTooSmall)?;
        command
            .extend_from_slice(b"\r\n")
            .map_err(|_| Error::BufferTooSmall)?;
        Ok(Self {
            command,
            payload: heapless::Vec::new(),
            timeout,
        })
    }

    /// A command followed by a payload.
    fn command_with_payload(cmd: &str, payload: &[u8], timeout: Duration) -> Result<Self> {
        let mut request = Self::command(cmd, timeout)?;
        request
            .payload
            .extend_from_slice(payload)
            .map_err(|_| Error::BufferTooSmall)?;
        Ok(request)
    }
}

// AT transaction result types live in [`crate::at::reply`] so that the other
// clients can share them; re-exported here for compatibility.
pub use crate::at::reply::{AtOutput, AtStatus};

/// Shared driver state.
///
/// Holds the embassy-net channel state and the AT request/response plumbing.
/// Create one per module, statically.
pub struct State<const N_RX: usize, const N_TX: usize> {
    ch: ch::State<N_RX, N_TX>,
    at_req: Channel<CriticalSectionRawMutex, AtRequest, 1>,
    at_resp: Signal<CriticalSectionRawMutex, AtOutput>,
}

impl<const N_RX: usize, const N_TX: usize> State<N_RX, N_TX> {
    /// Create the state.
    pub const fn new() -> Self {
        Self {
            ch: ch::State::new(),
            at_req: Channel::new(),
            at_resp: Signal::new(),
        }
    }
}

impl<const N_RX: usize, const N_TX: usize> Default for State<N_RX, N_TX> {
    fn default() -> Self {
        Self::new()
    }
}

/// Wi-Fi control plane: send AT commands over the same SPI link as the data.
pub struct Control<'d> {
    at_req: &'d Channel<CriticalSectionRawMutex, AtRequest, 1>,
    at_resp: &'d Signal<CriticalSectionRawMutex, AtOutput>,
    /// Lets the control plane report Wi-Fi link state to embassy-net.
    state: ch::StateRunner<'d>,
    timeout: Duration,
}

impl<'d> Control<'d> {
    /// Override the per-transaction timeout.
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Send an AT command and wait for its terminal response.
    ///
    /// `cmd` is the command without a line ending; `\r\n` is appended.
    pub async fn at(&self, cmd: &str) -> Result<AtOutput> {
        self.exchange(AtRequest::command(cmd, self.timeout)?).await
    }

    /// Send an AT command whose payload follows the module's `>` prompt.
    ///
    /// Used by `AT+OTASEND` (firmware update), `AT+BLEGATTSNTFY` and friends.
    pub async fn at_with_payload(&self, cmd: &str, payload: &[u8]) -> Result<AtOutput> {
        self.exchange(AtRequest::command_with_payload(cmd, payload, self.timeout)?)
            .await
    }

    async fn exchange(&self, request: AtRequest) -> Result<AtOutput> {
        // The wait here must use the request's own budget, not the one-shot
        // control timeout: `connect` asks for far longer than a plain command,
        // and the runner would otherwise still be collecting a response that
        // this side has already given up on.
        let timeout = request.timeout;

        // Drop any stale response left by a previous timed-out transaction.
        self.at_resp.reset();
        self.at_req.try_send(request).map_err(|_| Error::Busy)?;

        match with_timeout(timeout + RESPONSE_GRACE, self.at_resp.wait()).await {
            Ok(out) => Ok(out),
            Err(_) => Err(Error::Timeout),
        }
    }

    /// Join an access point (`AT+CWJAP`).
    ///
    /// On success the channel link state is raised to [`LinkState::Up`], which
    /// is what makes embassy-net start using the interface (DHCP, etc.).
    pub async fn connect(&self, ssid: &str, password: &str) -> Result<()> {
        use core::fmt::Write as _;
        let mut cmd = heapless::String::<160>::new();
        write!(cmd, "AT+CWJAP=\"{}\",\"{}\",", ssid, password).map_err(|_| Error::BufferTooSmall)?;
        // Associating takes seconds; give it its own budget rather than the
        // default command timeout.
        let out = self
            .exchange(AtRequest::command(&cmd, WIFI_JOIN_TIMEOUT)?)
            .await?;
        if !out.is_ok() {
            return Err(Error::AtCommandFailed);
        }
        self.state.set_link_state(LinkState::Up);
        Ok(())
    }

    /// Disconnect from the current access point (`AT+CWQAP`).
    pub async fn disconnect(&self) -> Result<()> {
        self.check("AT+CWQAP").await?;
        self.state.set_link_state(LinkState::Down);
        Ok(())
    }

    /// List nearby access points (`AT+CWLAP=<type>,<ssid>,<mac>,<chan>`).
    ///
    /// The results are returned as raw information lines in
    /// [`AtOutput::lines`].
    pub async fn scan(&self) -> Result<AtOutput> {
        self.exchange(AtRequest::command("AT+CWLAP=0,,,0", SCAN_TIMEOUT)?).await
    }

    /// Enable or disable IPv6 on the station interface (`AT+CIPV6`).
    pub async fn set_ipv6(&self, enable: bool) -> Result<()> {
        self.check(if enable { "AT+CIPV6=1" } else { "AT+CIPV6=0" })
            .await
    }

    /// Send a command and require an `OK`.
    async fn check(&self, cmd: &str) -> Result<()> {
        let out = self.at(cmd).await?;
        if out.is_ok() {
            Ok(())
        } else {
            Err(Error::AtCommandFailed)
        }
    }
}

impl<'d> crate::at::transport::AtTransport for Control<'d> {
    async fn at(&self, command: &str) -> Result<AtOutput> {
        Control::at(self, command).await
    }

    async fn at_with_payload(&self, command: &str, payload: &[u8]) -> Result<AtOutput> {
        Control::at_with_payload(self, command, payload).await
    }
}

/// Background task that owns the SPI engine.
///
/// It multiplexes three sources onto the one physical link: AT requests from
/// [`Control`], outbound Ethernet frames from the network stack, and the
/// module's RDY line.
pub struct Runner<'d, SPI, CS, const N_RX: usize, const N_TX: usize> {
    engine: Engine<SPI, CS>,
    ch: ch::Runner<'d>,
    at_req: &'d Channel<CriticalSectionRawMutex, AtRequest, 1>,
    at_resp: &'d Signal<CriticalSectionRawMutex, AtOutput>,
    ready: &'static Signal<CriticalSectionRawMutex, ()>,
    rx: [u8; RX_SCRATCH],
    // AT transaction scratch.
    at_buf: LineBuffer,
    at_lines: heapless::Vec<LineBuffer, AT_LINES_MAX>,
    at_status: Option<AtStatus>,
    at_prompt: bool,
    at_active: bool,
}

/// Create a driver, its background runner and the Wi-Fi control handle.
///
/// * `spi`/`cs` — the raw SPI bus and the **active-high** chip-select pin.
/// * `ready` — signalled by the RDY EXTI handler on every rising edge.
/// * `hdr_ack` — signalled on the falling edge.
/// * `rdy_level` — mirrors the RDY pin level; the handler must keep it in sync
///   on both edges.
/// * `mac` — the station MAC address, usually read with `AT+CIPSTAMAC?`.
#[allow(clippy::too_many_arguments)]
pub fn new<'d, SPI, CS, const N_RX: usize, const N_TX: usize>(
    spi: SPI,
    cs: CS,
    ready: &'static Signal<CriticalSectionRawMutex, ()>,
    hdr_ack: &'static Signal<CriticalSectionRawMutex, ()>,
    rdy_level: &'static AtomicBool,
    state: &'d mut State<N_RX, N_TX>,
    mac: [u8; 6],
) -> (
    WifiDevice<'d>,
    Runner<'d, SPI, CS, N_RX, N_TX>,
    Control<'d>,
)
where
    SPI: SpiBus,
    CS: OutputPin,
{
    let State { ch, at_req, at_resp } = state;
    let (ch_runner, device) = ch::new(ch, HardwareAddress::Ethernet(mac), MTU);
    let state_runner = ch_runner.state_runner();

    let at_req: &Channel<_, _, 1> = at_req;
    let at_resp: &Signal<_, _> = at_resp;

    let runner = Runner {
        engine: Engine::new(spi, cs, ready, hdr_ack, rdy_level),
        ch: ch_runner,
        at_req,
        at_resp,
        ready,
        rx: [0; RX_SCRATCH],
        at_buf: LineBuffer::new(),
        at_lines: heapless::Vec::new(),
        at_status: None,
        at_prompt: false,
        at_active: false,
    };
    let control = Control {
        at_req,
        at_resp,
        state: state_runner,
        timeout: AT_TIMEOUT,
    };

    (device, runner, control)
}

impl<'d, SPI, CS, const N_RX: usize, const N_TX: usize> Runner<'d, SPI, CS, N_RX, N_TX>
where
    SPI: SpiBus,
    CS: OutputPin,
{
    /// Run forever, servicing AT traffic and Ethernet frames.
    pub async fn run(mut self) -> ! {
        loop {
            match select3(self.at_req.receive(), self.ch.tx(), self.ready.wait()).await {
                Either3::First(cmd) => self.at_transaction(cmd).await,
                Either3::Second(buf) => {
                    let outbound = Outbound {
                        traffic_type: TrafficType::NetworkSta,
                        payload: &buf[..],
                    };
                    let res = self.engine.exchange(Some(outbound), &mut self.rx).await;
                    drop(buf);
                    self.after_exchange(res).await;
                }
                Either3::Third(()) => {
                    let res = self.engine.exchange(None, &mut self.rx).await;
                    self.after_exchange(res).await;
                }
            }
        }
    }

    /// Handle the result of one exchange, backing off briefly on error.
    async fn after_exchange(&mut self, res: Result<Received>) {
        match res {
            Ok(recv) => self.on_received(recv),
            Err(_) => Timer::after(Duration::from_millis(5)).await,
        }
    }

    /// Route one received frame by its traffic type.
    fn on_received(&mut self, recv: Received) {
        // Bring-up tracing: EVERY frame, with its decoded traffic type and raw
        // payload. This settles whether the module's command replies are typed
        // as AtCommand: if they are not, they are handed to the network stack
        // while the AT parser waits forever — which matches the symptom exactly
        // (the boot banner is parsed, command replies never are).
        let shown = recv.len.min(64);
        defmt::trace!(
            "rx: type={:?} len={} bytes={=[u8]}",
            recv.traffic_type(),
            recv.len,
            &self.rx[..shown]
        );

        match recv.traffic_type() {
            Some(TrafficType::NetworkSta) | Some(TrafficType::NetworkAp) => {
                self.deliver_ethernet(recv.len);
            }
            Some(TrafficType::AtCommand) => {
                self.feed_at(recv.len);
            }
            _ => {}
        }
    }

    /// Hand an Ethernet frame to the embassy-net channel.
    fn deliver_ethernet(&mut self, len: usize) {
        let Some(mut packet) = PacketBuf::try_new() else {
            // Pool exhausted: drop. The stack will retransmit.
            return;
        };
        let n = len.min(packet.capacity());
        packet.storage_mut()[..n].copy_from_slice(&self.rx[..n]);
        packet.set_len(n);
        let _ = self.ch.try_rx(packet);
    }

    /// Feed received AT bytes into the line parser.
    fn feed_at(&mut self, len: usize) {
        if !self.at_active {
            // Unsolicited event outside a transaction; ignored for now.
            return;
        }
        for i in 0..len {
            let b = self.rx[i];
            if b == b'\n' {
                self.finish_at_line();
            } else if b != b'\r' {
                let _ = self.at_buf.push(b as char);
            }
        }
    }

    /// Parse and route one complete AT line.
    fn finish_at_line(&mut self) {
        if self.at_buf.is_empty() {
            return;
        }
        // Bring-up tracing: what the module actually said, line by line.
        defmt::trace!("at line: |{}|", self.at_buf.as_str());
        if let Ok(Some(resp)) = parser::parse_line(&self.at_buf) {
            match resp {
                AtResponse::Ok => self.at_status = Some(AtStatus::Ok),
                AtResponse::Error => self.at_status = Some(AtStatus::Error),
                AtResponse::ReadyPrompt => self.at_prompt = true,
                _ => {
                    if self.at_status.is_none() && !self.at_prompt {
                        let _ = self.at_lines.push(self.at_buf.clone());
                    }
                }
            }
        }
        self.at_buf.clear();
    }

    /// Run one AT transaction and collect its response.
    ///
    /// If the request carries a payload, it is sent once the module answers
    /// with the `>` prompt.
    async fn at_transaction(&mut self, request: AtRequest) {
        let AtRequest {
            command,
            mut payload,
            timeout,
        } = request;

        self.at_lines.clear();
        self.at_status = None;
        self.at_prompt = false;
        self.at_buf.clear();
        self.at_active = true;

        // NOTE: there is deliberately no "drain stale frames" step here. An
        // earlier version flushed the queue with receive-only exchanges, i.e. it
        // sent EMPTY AT frames before every command, which jammed the module's
        // state machine so it stopped answering entirely. at_transaction already
        // copes with a leading banner or event on its own: it keeps reading until
        // it sees a terminal OK/ERROR, collecting anything else as a data line.

        // Bring-up tracing: which command starts, with which budget. The command
        // is truncated so a password never reaches the log.
        defmt::trace!(
            "at start: cmd={=[u8]} len={} timeout={}ms",
            &command[..command.len().min(16)],
            command.len(),
            timeout.as_millis()
        );

        // Send the command and dispatch anything that arrived with it.
        match self
            .engine
            .exchange(Some(Outbound::at(&command)), &mut self.rx)
            .await
        {
            Ok(recv) => {
                defmt::trace!("at send ok: rx_len={}", recv.len);
                self.on_received(recv);
            }
            // Previously swallowed: if the command frame never got out, the
            // module cannot answer and the transaction just times out silently.
            Err(e) => defmt::trace!("at send ERR: {:?}", e),
        }

        let deadline = Instant::now() + timeout;
        while self.at_status.is_none() {
            // A pending payload goes out as soon as the module prompts.
            if self.at_prompt && !payload.is_empty() {
                if let Ok(recv) = self
                    .engine
                    .exchange(Some(Outbound::at(&payload)), &mut self.rx)
                    .await
                {
                    self.on_received(recv);
                }
                payload.clear();
                self.at_prompt = false;
                continue;
            }

            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let remaining = deadline - now;
            match with_timeout(remaining, self.engine.exchange(None, &mut self.rx)).await {
                Ok(Ok(recv)) => self.on_received(recv),
                // A transient engine error — no RDY just now, or a malformed
                // frame — must NOT abort the transaction. Breaking here meant a
                // single glitch truncated the response and the command was then
                // judged on whatever had arrived so far, usually nothing.
                Ok(Err(_)) => Timer::after(Duration::from_millis(2)).await,
                // The overall deadline elapsed.
                Err(_) => break,
            }
        }

        defmt::trace!(
            "at transaction done: status={} lines={}",
            match self.at_status {
                Some(AtStatus::Ok) => "Ok",
                Some(AtStatus::Error) => "Error",
                Some(AtStatus::Timeout) => "Timeout",
                None => "None",
            },
            self.at_lines.len()
        );

        self.at_active = false;
        let lines = core::mem::take(&mut self.at_lines);
        self.at_resp.signal(AtOutput {
            status: self.at_status.take().unwrap_or(AtStatus::Timeout),
            lines,
        });
    }
}
