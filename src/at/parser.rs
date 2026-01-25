//! AT response parser

use crate::error::{Error, Result};
use crate::types::*;
use heapless::{String, Vec};

/// Maximum line length for parsing
pub const MAX_LINE_LEN: usize = 512;

/// Line buffer type
pub type LineBuffer = String<MAX_LINE_LEN>;

/// AT response type
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AtResponse {
    /// OK response
    Ok,
    /// ERROR response
    Error,
    /// SEND OK response
    SendOk,
    /// SEND FAIL response
    SendFail,
    /// Ready prompt (">")
    ReadyPrompt,
    /// Data line with prefix and content
    Data {
        prefix: LineBuffer,
        content: LineBuffer,
    },
    /// Raw data
    Raw(LineBuffer),
    /// +IPD notification header (link_id, length)
    /// The actual data bytes follow this notification
    IpdHeader { link_id: u8, length: usize },
}

/// Parse a single line of AT response
pub fn parse_line(line: &str) -> Result<Option<AtResponse>> {
    let trimmed = line.trim();

    if trimmed.is_empty() {
        return Ok(None);
    }

    // Check for standard responses
    match trimmed {
        "OK" => return Ok(Some(AtResponse::Ok)),
        "ERROR" => return Ok(Some(AtResponse::Error)),
        "SEND OK" => return Ok(Some(AtResponse::SendOk)),
        "SEND FAIL" => return Ok(Some(AtResponse::SendFail)),
        ">" => return Ok(Some(AtResponse::ReadyPrompt)),
        _ => {}
    }

    // Check for +IPD notification (special handling for binary data)
    if trimmed.starts_with("+IPD,") {
        // Parse +IPD,<link_id>,<length>:
        if let Some(colon_pos) = trimmed.find(':') {
            let params = &trimmed[5..colon_pos]; // Skip "+IPD,"
            let parts = parse_csv(params);

            if parts.len() >= 2 {
                if let (Ok(link_id), Ok(length)) = (parse_int(&parts[0]), parse_int(&parts[1])) {
                    return Ok(Some(AtResponse::IpdHeader {
                        link_id: link_id as u8,
                        length: length as usize,
                    }));
                }
            }
        }
    }

    // Check for other prefixed data (e.g., "+CWLAP:...")
    if trimmed.starts_with('+') {
        if let Some(colon_pos) = trimmed.find(':') {
            let prefix = &trimmed[0..colon_pos];
            let content = &trimmed[colon_pos + 1..].trim();

            let mut prefix_buf = LineBuffer::new();
            prefix_buf
                .push_str(prefix)
                .map_err(|_| Error::BufferTooSmall)?;

            let mut content_buf = LineBuffer::new();
            content_buf
                .push_str(content)
                .map_err(|_| Error::BufferTooSmall)?;

            return Ok(Some(AtResponse::Data {
                prefix: prefix_buf,
                content: content_buf,
            }));
        }
    }

    // Raw data line
    let mut raw_buf = LineBuffer::new();
    raw_buf
        .push_str(trimmed)
        .map_err(|_| Error::BufferTooSmall)?;
    Ok(Some(AtResponse::Raw(raw_buf)))
}

/// Parse comma-separated values
pub fn parse_csv(input: &str) -> Vec<LineBuffer, 16> {
    let mut result = Vec::new();
    let mut current = LineBuffer::new();
    let mut in_quotes = false;
    let mut escape_next = false;

    for ch in input.chars() {
        if escape_next {
            let _ = current.push(ch);
            escape_next = false;
            continue;
        }

        match ch {
            '\\' => {
                escape_next = true;
            }
            '"' => {
                in_quotes = !in_quotes;
            }
            ',' if !in_quotes => {
                if result.push(current.clone()).is_err() {
                    break;
                }
                current.clear();
            }
            _ => {
                let _ = current.push(ch);
            }
        }
    }

    // Push last field
    let _ = result.push(current);

    result
}

/// Remove surrounding quotes from a string
pub fn unquote(s: &str) -> &str {
    let trimmed = s.trim();
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

/// Parse integer from string
pub fn parse_int(s: &str) -> Result<i32> {
    s.trim().parse::<i32>().map_err(|_| Error::ParseError)
}

/// Parse IP address from string (format: "192.168.1.1")
pub fn parse_ip(s: &str) -> Result<Ipv4Address> {
    let parts: Vec<&str, 4> = s.split('.').collect();
    if parts.len() != 4 {
        return Err(Error::ParseError);
    }

    let octets: Result<Vec<u8, 4>> = parts
        .iter()
        .map(|p| p.parse::<u8>().map_err(|_| Error::ParseError))
        .collect();

    let octets = octets?;
    Ok(Ipv4Address::new(octets[0], octets[1], octets[2], octets[3]))
}

/// Parse MAC address from string (format: "aa:bb:cc:dd:ee:ff")
pub fn parse_mac(s: &str) -> Result<MacAddress> {
    let parts: Vec<&str, 6> = s.split(':').collect();
    if parts.len() != 6 {
        return Err(Error::ParseError);
    }

    let mut bytes = [0u8; 6];
    for (i, part) in parts.iter().enumerate() {
        bytes[i] = u8::from_str_radix(part, 16).map_err(|_| Error::ParseError)?;
    }

    Ok(MacAddress::new(bytes))
}

/// Parse scan result from AT+CWLAP response
/// Format: +CWLAP:(ecn,ssid,rssi,mac,channel)
pub fn parse_scan_result(content: &str) -> Result<ScanResult> {
    // Remove parentheses
    let trimmed = content.trim();
    let inner = if trimmed.starts_with('(') && trimmed.ends_with(')') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    let fields = parse_csv(inner);
    if fields.len() < 5 {
        return Err(Error::ParseError);
    }

    // Parse security type
    let security = match parse_int(&fields[0])? {
        0 => WiFiSecurityType::Open,
        1 => WiFiSecurityType::Wep,
        2 => WiFiSecurityType::WpaPsk,
        3 => WiFiSecurityType::Wpa2Psk,
        4 => WiFiSecurityType::WpaWpa2Psk,
        5 => WiFiSecurityType::Wpa2Enterprise,
        6 => WiFiSecurityType::Wpa3,
        _ => WiFiSecurityType::Open,
    };

    // Parse SSID (remove quotes)
    let ssid_str = unquote(&fields[1]);
    let mut ssid = Ssid::new();
    ssid.push_str(ssid_str).map_err(|_| Error::ParseError)?;

    // Parse RSSI
    let rssi = parse_int(&fields[2])? as i8;

    // Parse MAC address (remove quotes)
    let mac_str = unquote(&fields[3]);
    let bssid = parse_mac(mac_str)?.0;

    // Parse channel
    let channel = parse_int(&fields[4])? as u8;

    Ok(ScanResult {
        ssid,
        bssid,
        channel,
        rssi,
        security,
    })
}

/// Parse IP config from AT+CIPSTA response
/// Format: +CIPSTA:ip:"192.168.1.100"
///         +CIPSTA:gateway:"192.168.1.1"
///         +CIPSTA:netmask:"255.255.255.0"
pub fn parse_ip_config_line(
    prefix: &str,
    content: &str,
) -> Result<Option<(IpConfigField, Ipv4Address)>> {
    let fields = parse_csv(content);
    if fields.is_empty() {
        return Ok(None);
    }

    let field_name = unquote(&fields[0]);
    let field_type = match field_name {
        "ip" => IpConfigField::Ip,
        "gateway" => IpConfigField::Gateway,
        "netmask" => IpConfigField::Netmask,
        _ => return Ok(None),
    };

    if fields.len() < 2 {
        return Err(Error::ParseError);
    }

    let ip_str = unquote(&fields[1]);
    let ip = parse_ip(ip_str)?;

    Ok(Some((field_type, ip)))
}

/// IP config field type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpConfigField {
    Ip,
    Gateway,
    Netmask,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_line_ok() {
        let result = parse_line("OK").unwrap();
        assert_eq!(result, Some(AtResponse::Ok));
    }

    #[test]
    fn test_parse_line_error() {
        let result = parse_line("ERROR").unwrap();
        assert_eq!(result, Some(AtResponse::Error));
    }

    #[test]
    fn test_parse_csv() {
        let result = parse_csv("1,\"test\",3");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].as_str(), "1");
        assert_eq!(result[1].as_str(), "\"test\"");
        assert_eq!(result[2].as_str(), "3");
    }

    #[test]
    fn test_unquote() {
        assert_eq!(unquote("\"test\""), "test");
        assert_eq!(unquote("test"), "test");
        assert_eq!(unquote("\""), "\"");
    }

    #[test]
    fn test_parse_ip() {
        let ip = parse_ip("192.168.1.100").unwrap();
        assert_eq!(ip.octets(), [192, 168, 1, 100]);
    }
}
