//! Shared AT transaction result types.
//!
//! These live here, rather than in the T02 driver, so that higher-level
//! clients (Wi-Fi, BLE, HTTP, …) can be generic over whichever transport they
//! run on.

use crate::at::parser::LineBuffer;

/// Maximum information lines kept per AT transaction.
pub const AT_LINES_MAX: usize = 16;

/// Terminal status of an AT transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AtStatus {
    /// The module answered `OK`.
    Ok,
    /// The module answered `ERROR`.
    Error,
    /// No terminal response arrived before the timeout.
    Timeout,
}

/// Result of an AT transaction.
#[derive(Debug, Clone)]
pub struct AtOutput {
    /// How the transaction ended.
    pub status: AtStatus,
    /// Information lines the module emitted before `OK`/`ERROR`.
    pub lines: heapless::Vec<LineBuffer, AT_LINES_MAX>,
}

impl AtOutput {
    /// Whether the module answered `OK`.
    pub fn is_ok(&self) -> bool {
        self.status == AtStatus::Ok
    }

    /// The first information line starting with `prefix`.
    ///
    /// Modules prefix information lines with the command name and a colon, for
    /// example `+BLENAME:sensor`; the returned slice is everything after
    /// `prefix`.
    pub fn value_after(&self, prefix: &str) -> Option<&str> {
        self.lines
            .iter()
            .find_map(|line| line.as_str().strip_prefix(prefix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(status: AtStatus, lines: &[&str]) -> AtOutput {
        let mut v = heapless::Vec::new();
        for line in lines {
            let mut b = LineBuffer::new();
            b.push_str(line).unwrap();
            v.push(b).unwrap();
        }
        AtOutput { status, lines: v }
    }

    #[test]
    fn is_ok_reflects_status() {
        assert!(out(AtStatus::Ok, &[]).is_ok());
        assert!(!out(AtStatus::Error, &[]).is_ok());
    }

    #[test]
    fn value_after_strips_prefix() {
        let o = out(AtStatus::Ok, &["+BLENAME:my sensor"]);
        assert_eq!(o.value_after("+BLENAME:"), Some("my sensor"));
        assert_eq!(o.value_after("+BLEADDR:"), None);
    }
}
