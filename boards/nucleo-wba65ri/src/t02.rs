//! T02 helpers: **this MCU** owns TCP/IP. The module is only a Wi-Fi MAC/PHY that
//! carries raw Ethernet frames, and `embassy-net` (the xarxa flavour) runs the
//! whole stack on the STM32 (DHCP, DNS, TCP, UDP).
//!
//! Included from the T02 bins as `#[path = "../t02.rs"] mod t02;` next to
//! `common.rs`.

#![allow(dead_code)]

use core::fmt::Write as _;

use defmt::{error, info, unwrap, warn};
use embassy_executor::Spawner;
use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::TcpSocket;
use embassy_net::udp::UdpSocket;
use embassy_net::wire::IpAddr;
use embassy_net::{Stack, StackStorage, StaticPool};
use embassy_time::{with_timeout, Duration, Timer};
use embedded_io_async::Write as _;
use static_cell::StaticCell;

use crate::common::{self, Started};
use st67w611::net::xarxa::HardwareAddress;

/// Packet pool for the whole stack: 1514 bytes = Ethernet MTU.
static POOL: StaticPool<1514, 16> = StaticPool::new();

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static>) {
    runner.run().await
}

/// Start the `embassy-net` stack over the module and get a DHCP address.
///
/// Order matters (it is the order `main.rs` uses): seed and start the stack,
/// learn the module's own station MAC (it filters inbound unicast by it), attach
/// the interface with that MAC and start DHCP, and only then associate.
///
/// Returns the stack and the IPv4 address it was given.
pub async fn bring_up(
    spawner: Spawner,
    started: Started,
) -> Option<(Stack<'static>, heapless::String<16>)> {
    let Started {
        control,
        device,
        mut rng,
        ..
    } = started;

    // The seed feeds TCP initial sequence numbers, so take it from the hardware
    // RNG rather than a constant.
    let mut seed_bytes = [0u8; 8];
    unwrap!(rng.fill_bytes(&mut seed_bytes).await);
    let seed = u64::from_le_bytes(seed_bytes);

    static STACK_STORAGE: StaticCell<StackStorage<'static>> = StaticCell::new();
    static DEVICE: StaticCell<st67w611::net::xarxa::WifiDevice<'static>> = StaticCell::new();
    let (stack, runner) = Stack::new(STACK_STORAGE.init(StackStorage::new()), &POOL, seed);
    spawner.spawn(unwrap!(net_task(runner)));

    let mac = match control.station_mac().await {
        Ok(mac) => mac,
        Err(e) => {
            error!("could not read the module's station MAC: {:?}", e);
            return None;
        }
    };
    control.set_hardware_address(HardwareAddress::Ethernet(mac));
    let iface = unwrap!(stack.add_iface_borrowed(DEVICE.init(device)));
    unwrap!(iface.set_dhcpv4(Some(Default::default())));
    // Pin public resolvers ahead of whatever DHCP advertises.
    stack.set_dns_servers(&[IpAddr::v4(1, 1, 1, 1), IpAddr::v4(9, 9, 9, 9)]);

    if !common::join_ap(control).await {
        return None;
    }
    // Raises the link, which is what makes the stack start DHCP. (A join through
    // `Control::connect` would do this itself; the stored-network path does not.)
    control.set_link(true);

    for _ in 0..60 {
        if iface.is_config_v4_up() {
            for addr in iface.ip_addrs() {
                let mut s = heapless::String::<40>::new();
                let _ = write!(s, "{}", addr.cidr.address());
                // Skip IPv6 and the 169.254/16 link-local placeholder.
                if !s.contains(':') && !s.starts_with("169.254.") {
                    let mut ip = heapless::String::new();
                    let _ = ip.push_str(&s);
                    return Some((stack, ip));
                }
            }
        }
        Timer::after_millis(500).await;
    }
    error!("no DHCP address after 30 s");
    None
}

/// Resolve `host` to its first IPv4 address, using the host-side DNS client.
pub async fn resolve(stack: Stack<'static>, host: &str) -> Option<IpAddr> {
    match stack.dns_query(host, DnsQueryType::A).await {
        Ok(addrs) if !addrs.is_empty() => Some(addrs[0]),
        Ok(_) => None,
        Err(e) => {
            warn!("dns: {:?}", e);
            None
        }
    }
}

/// Seconds between the NTP epoch (1900) and the Unix epoch (1970).
const NTP_UNIX_OFFSET: u64 = 2_208_988_800;

/// SNTP over a host-side UDP socket. Returns Unix seconds.
///
/// The request is the classic 48-byte SNTP packet (LI=0, VN=3, mode=3); the reply
/// carries the transmit timestamp at bytes 40..48, big-endian 32.32 fixed point.
pub async fn sntp(stack: Stack<'static>, host: &str) -> Option<u64> {
    const NTP_PORT: u16 = 123;
    let addr = resolve(stack, host).await?;
    let mut sock = UdpSocket::new(stack).ok()?;
    sock.bind(0u16, (addr, NTP_PORT)).ok()?;

    let mut req = [0u8; 48];
    req[0] = 0x1B;
    sock.send_to(&req, (addr, NTP_PORT)).await.ok()?;

    let mut buf = [0u8; 48];
    let n = with_timeout(
        Duration::from_secs(5),
        sock.recv_from_with(|data, _meta| {
            let n = data.len().min(buf.len());
            buf[..n].copy_from_slice(&data[..n]);
            n
        }),
    )
    .await
    .ok()?
    .ok()?;
    if n < 48 {
        return None;
    }
    let secs = u32::from_be_bytes([buf[40], buf[41], buf[42], buf[43]]) as u64;
    (secs != 0).then(|| secs.wrapping_sub(NTP_UNIX_OFFSET))
}

/// Plain HTTP GET over a host-side TCP socket. On success `status` holds the
/// response's status line. Returns a short reason on failure.
pub async fn http_get(
    stack: Stack<'static>,
    host: &str,
    path: &str,
    status: &mut heapless::String<96>,
) -> Result<(), &'static str> {
    let addr = resolve(stack, host).await.ok_or("dns failed")?;

    let mut rx = [0u8; 1024];
    let mut tx = [0u8; 256];
    let mut sock = TcpSocket::new(stack, &mut rx, &mut tx).map_err(|_| "no free socket")?;

    let mut req = heapless::String::<256>::new();
    let _ = write!(
        req,
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: st67w611-t02\r\nConnection: close\r\n\r\n",
        path, host
    );

    let mut buf = [0u8; 256];
    let mut n = 0usize;
    let exchange = async {
        sock.connect((addr, 80u16))
            .await
            .map_err(|_| "connect failed")?;
        sock.write_all(req.as_bytes())
            .await
            .map_err(|_| "write failed")?;
        // Only the status line is needed, so stop at the first line break.
        while n < buf.len() && !buf[..n].contains(&b'\n') {
            match sock.read(&mut buf[n..]).await {
                Ok(0) | Err(_) => break,
                Ok(k) => n += k,
            }
        }
        Ok::<(), &'static str>(())
    };
    let result = with_timeout(Duration::from_secs(8), exchange).await;
    sock.close();
    match result {
        Err(_) => return Err("timed out"),
        Ok(Err(why)) => return Err(why),
        Ok(Ok(())) => {}
    }

    let text = core::str::from_utf8(&buf[..n]).unwrap_or("");
    let line = text.lines().next().unwrap_or("");
    if line.starts_with("HTTP/1.") {
        let _ = status.push_str(&line[..line.len().min(95)]);
        info!("  (read {} bytes)", n);
        Ok(())
    } else {
        Err("no HTTP response")
    }
}
