//! Wi-Fi support.
//!
//! The transport-generic client is [`client::WiFi`], re-exported as [`WiFi`].
//! It replaces the old T01 `WiFiManager`, which was a duplicate of the same AT
//! commands.
//!
//! Unsolicited Wi-Fi events (`WIFI GOT IP`, disconnect notifications, …) are
//! not part of the request/response AT transport; they are delivered by the
//! driver ([`Driver::next_wifi_event`](crate::Driver::next_wifi_event)).

pub mod client;

pub use client::WiFi;
