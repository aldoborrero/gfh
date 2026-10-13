//! Read a YubiKey's serial over its FIDO HID interface.
//!
//! The serial is what the config is keyed on, and reading it over PC/SC needs
//! pcscd and the CCID interface. On a YubiKey shared over USB/IP (WSL), pcscd
//! polling CCID while the agent signs over FIDO corrupts the signature, so read
//! it from the FIDO interface instead, the one signing already uses. Yubico's
//! vendor command READ_CONFIG returns the device info that `ykman` reads over
//! FIDO, which includes the serial.

use anyhow::{Context, Result, bail};
use hidapi::{HidApi, HidDevice, HidResult};
use std::{
    collections::hash_map::RandomState,
    ffi::CString,
    hash::{BuildHasher, Hasher},
    time::{Duration, Instant},
};

const PACKET: usize = 64;
const BROADCAST_CID: [u8; 4] = [0xff; 4];

const CTAPHID_INIT: u8 = 0x86;
const CTAPHID_KEEPALIVE: u8 = 0xbb;
const CTAPHID_ERROR: u8 = 0xbf;
/// Yubico vendor command: read the management application's device info.
const CTAPHID_YUBIKEY_READ_CONFIG: u8 = 0xc2;

/// CTAPHID_ERROR code for a command the device does not implement.
const ERR_INVALID_CMD: u8 = 0x01;

/// Device info tag holding the serial, a big-endian u32.
const TAG_SERIAL: u8 = 0x02;

/// The two HID operations a request needs, so tests can stand in a fake device.
trait Hid {
    fn write(&self, report: &[u8]) -> HidResult<usize>;
    fn read_timeout(&self, buf: &mut [u8], timeout_ms: i32) -> HidResult<usize>;
}

impl Hid for HidDevice {
    fn write(&self, report: &[u8]) -> HidResult<usize> {
        HidDevice::write(self, report)
    }

    fn read_timeout(&self, buf: &mut [u8], timeout_ms: i32) -> HidResult<usize> {
        HidDevice::read_timeout(self, buf, timeout_ms)
    }
}

/// The serial of the YubiKey whose FIDO HID interface is at `path`.
///
/// `Ok(None)` means the key answered but does not report a serial this way:
/// firmware without READ_CONFIG over FIDO (YubiKey 4 and older), or a serial
/// configured as not visible. Callers fall back to PC/SC for those. Any other
/// failure, a busy key included, is an error.
///
/// hidapi allows one `HidApi` at a time, and ctap-hid-fido2 opens its own for
/// every call, so do not call into ctap-hid-fido2 while `api` is alive.
///
/// `timeout` bounds each request as a whole, not each read: other clients'
/// traffic on the device (an agent waiting for a touch sends keepalives) must
/// not keep us waiting forever. A healthy key answers in milliseconds; the
/// limit only matters when it never answers, e.g. a stale USB/IP attachment,
/// which would otherwise hang the git command waiting on gfh.
pub fn read_serial(api: &HidApi, path: &str, timeout: Duration) -> Result<Option<u32>> {
    let path = CString::new(path).context("HID path contains a NUL byte")?;
    let device = api
        .open_path(&path)
        .with_context(|| format!("could not open {}", path.to_string_lossy()))?;
    query_serial(&device, timeout)
}

fn query_serial(device: &impl Hid, timeout: Duration) -> Result<Option<u32>> {
    // The nonce only has to tell our INIT response from another client's;
    // RandomState is randomly keyed per process, which is enough for that.
    let nonce = RandomState::new().build_hasher().finish().to_be_bytes();
    let init = transact(device, BROADCAST_CID, CTAPHID_INIT, &nonce, timeout)?;
    let cid: [u8; 4] = init
        .get(8..12)
        .and_then(|c| c.try_into().ok())
        .context("CTAPHID_INIT response too short")?;

    match transact(device, cid, CTAPHID_YUBIKEY_READ_CONFIG, &[], timeout) {
        Ok(info) => Ok(parse_serial(&info)),
        Err(err) if err.downcast_ref::<CtapError>() == Some(&CtapError(ERR_INVALID_CMD)) => {
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

/// A CTAPHID_ERROR response, with its error code.
#[derive(Debug, PartialEq)]
struct CtapError(u8);

impl std::fmt::Display for CtapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            0x06 => write!(
                f,
                "the key is busy with another client (CTAPHID error 0x06)"
            ),
            code => write!(f, "the key returned CTAPHID error {code:#04x}"),
        }
    }
}

impl std::error::Error for CtapError {}

/// Send one CTAPHID request and return its response payload.
fn transact(
    device: &impl Hid,
    cid: [u8; 4],
    cmd: u8,
    data: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>> {
    // Every request this module sends fits in the initialization packet.
    assert!(data.len() <= PACKET - 7);

    // hidapi takes the report ID first; FIDO HID devices use none, so 0.
    let mut report = [0u8; PACKET + 1];
    report[1..5].copy_from_slice(&cid);
    report[5] = cmd;
    report[6..8].copy_from_slice(&(data.len() as u16).to_be_bytes());
    report[8..8 + data.len()].copy_from_slice(data);
    device.write(&report).context("HID write failed")?;

    let deadline = Instant::now() + timeout;
    let mut packet = [0u8; PACKET];
    let (len, mut payload) = loop {
        read_packet(device, &mut packet, deadline)?;
        // Every reader of the device sees every report: skip other channels,
        // the keepalives of a busy key, and, on the broadcast channel, INIT
        // responses meant for another client's nonce.
        if packet[..4] != cid
            || packet[4] == CTAPHID_KEEPALIVE
            || (cmd == CTAPHID_INIT && packet[4] == CTAPHID_INIT && packet[7..15] != *data)
        {
            continue;
        }
        if packet[4] == CTAPHID_ERROR {
            return Err(CtapError(packet[7]).into());
        }
        if packet[4] != cmd {
            bail!(
                "unexpected CTAPHID response {:#04x} to {cmd:#04x}",
                packet[4]
            );
        }
        let len = usize::from(u16::from_be_bytes([packet[5], packet[6]]));
        break (len, packet[7..].to_vec());
    };

    let mut seq = 0u8;
    while payload.len() < len {
        read_packet(device, &mut packet, deadline)?;
        if packet[..4] != cid {
            continue;
        }
        if packet[4] != seq {
            bail!("CTAPHID continuation packet out of sequence");
        }
        payload.extend_from_slice(&packet[5..]);
        seq += 1;
    }
    payload.truncate(len);
    Ok(payload)
}

fn read_packet(device: &impl Hid, packet: &mut [u8; PACKET], deadline: Instant) -> Result<()> {
    let left = deadline.saturating_duration_since(Instant::now());
    // hidapi reads a timeout of 0 as "do not wait", so round up to 1 ms.
    let ms = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
    match device.read_timeout(packet, ms) {
        Ok(0) => bail!("timed out waiting for the key"),
        Ok(_) if Instant::now() > deadline => bail!("timed out waiting for the key"),
        Ok(_) => Ok(()),
        Err(err) => Err(err).context("HID read failed"),
    }
}

/// The serial in a READ_CONFIG response: a length byte, then tag-length-value
/// entries.
fn parse_serial(response: &[u8]) -> Option<u32> {
    let (&len, rest) = response.split_first()?;
    let mut tlvs = rest.get(..usize::from(len))?;
    while let [tag, len, rest @ ..] = tlvs {
        let value = rest.get(..usize::from(*len))?;
        if *tag == TAG_SERIAL {
            return value.try_into().ok().map(u32::from_be_bytes);
        }
        tlvs = &rest[value.len()..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// A device that answers with canned packets, recording what it was sent.
    struct Fake {
        replies: RefCell<VecDeque<[u8; PACKET]>>,
        written: RefCell<Vec<Vec<u8>>>,
    }

    impl Fake {
        fn new(replies: Vec<[u8; PACKET]>) -> Self {
            Fake {
                replies: RefCell::new(replies.into()),
                written: RefCell::default(),
            }
        }
    }

    impl Hid for Fake {
        fn write(&self, report: &[u8]) -> HidResult<usize> {
            self.written.borrow_mut().push(report.to_vec());
            Ok(report.len())
        }

        fn read_timeout(&self, buf: &mut [u8], _timeout_ms: i32) -> HidResult<usize> {
            match self.replies.borrow_mut().pop_front() {
                Some(packet) => {
                    buf.copy_from_slice(&packet);
                    Ok(PACKET)
                }
                None => Ok(0),
            }
        }
    }

    fn init_packet(cid: [u8; 4], cmd: u8, len: usize, data: &[u8]) -> [u8; PACKET] {
        let mut p = [0u8; PACKET];
        p[..4].copy_from_slice(&cid);
        p[4] = cmd;
        p[5..7].copy_from_slice(&(len as u16).to_be_bytes());
        p[7..7 + data.len()].copy_from_slice(data);
        p
    }

    fn cont_packet(cid: [u8; 4], seq: u8, data: &[u8]) -> [u8; PACKET] {
        let mut p = [0u8; PACKET];
        p[..4].copy_from_slice(&cid);
        p[4] = seq;
        p[5..5 + data.len()].copy_from_slice(data);
        p
    }

    const T: Duration = Duration::from_secs(2);
    const CID: [u8; 4] = [1, 2, 3, 4];
    const OTHER: [u8; 4] = [9, 9, 9, 9];

    #[test]
    fn reassembles_continuation_and_skips_other_traffic() {
        let payload: Vec<u8> = (0..100).collect();
        let fake = Fake::new(vec![
            init_packet(OTHER, 0x83, 5, &[0; 5]),
            init_packet(CID, CTAPHID_KEEPALIVE, 1, &[1]),
            init_packet(CID, 0x83, payload.len(), &payload[..57]),
            cont_packet(OTHER, 0, &[0xee; 59]),
            cont_packet(CID, 0, &payload[57..]),
        ]);
        assert_eq!(transact(&fake, CID, 0x83, &[], T).unwrap(), payload);
        let written = fake.written.borrow();
        assert_eq!(written[0].len(), PACKET + 1);
        assert_eq!(written[0][..8], [0, 1, 2, 3, 4, 0x83, 0, 0]);
    }

    #[test]
    fn init_ignores_another_clients_nonce() {
        let nonce = [7u8; 8];
        let mut theirs = [0u8; 17];
        theirs[..8].copy_from_slice(&[8; 8]);
        let mut ours = [0u8; 17];
        ours[..8].copy_from_slice(&nonce);
        ours[8..12].copy_from_slice(&CID);
        let fake = Fake::new(vec![
            init_packet(BROADCAST_CID, CTAPHID_INIT, 17, &theirs),
            init_packet(BROADCAST_CID, CTAPHID_INIT, 17, &ours),
        ]);
        let init = transact(&fake, BROADCAST_CID, CTAPHID_INIT, &nonce, T).unwrap();
        assert_eq!(init[8..12], CID);
    }

    #[test]
    fn unsupported_command_is_no_serial_but_busy_is_an_error() {
        /// Answers INIT with whatever nonce was sent, then READ_CONFIG with
        /// CTAPHID_ERROR carrying `.1`.
        struct Echo(Fake, u8);

        impl Hid for Echo {
            fn write(&self, report: &[u8]) -> HidResult<usize> {
                if report[5] == CTAPHID_INIT {
                    let mut init = [0u8; 17];
                    init[..8].copy_from_slice(&report[8..16]);
                    init[8..12].copy_from_slice(&CID);
                    let mut replies = self.0.replies.borrow_mut();
                    replies.push_back(init_packet(BROADCAST_CID, CTAPHID_INIT, 17, &init));
                    replies.push_back(init_packet(CID, CTAPHID_ERROR, 1, &[self.1]));
                }
                self.0.write(report)
            }

            fn read_timeout(&self, buf: &mut [u8], ms: i32) -> HidResult<usize> {
                self.0.read_timeout(buf, ms)
            }
        }

        let unsupported = Echo(Fake::new(vec![]), ERR_INVALID_CMD);
        assert_eq!(query_serial(&unsupported, T).unwrap(), None);

        let busy = Echo(Fake::new(vec![]), 0x06);
        let err = query_serial(&busy, T).unwrap_err();
        assert!(err.to_string().contains("busy"), "{err:#}");
    }

    #[test]
    fn silence_times_out() {
        let fake = Fake::new(vec![]);
        let err = transact(&fake, CID, 0x83, &[], T).unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err:#}");
    }

    #[test]
    fn parses_serial_from_a_real_response() {
        // READ_CONFIG from a YubiKey 5C NFC, firmware 5.4.3.
        let response = hex(
            "2e0102033f030202330204013b087604010305030504030602000007010f\
             0801000d02033f0e02023b0a01010f0100",
        );
        assert_eq!(parse_serial(&response), Some(20_646_006));
    }

    #[test]
    fn no_serial_tag_is_none() {
        // Only a firmware version (tag 0x05).
        assert_eq!(parse_serial(&hex("050503050403")), None);
    }

    #[test]
    fn truncated_responses_are_none() {
        assert_eq!(parse_serial(&[]), None);
        // Length byte claims more than is there.
        assert_eq!(parse_serial(&hex("0a0204013b")), None);
        // Serial value shorter than its declared length.
        assert_eq!(parse_serial(&hex("040204013b")), None);
        // Serial of the wrong width.
        assert_eq!(parse_serial(&hex("0402020102")), None);
    }
}
