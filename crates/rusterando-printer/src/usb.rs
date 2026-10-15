//! Userspace USB printer driver (usbfs via `nusb`, no C deps), for kernels
//! without the `usblp` driver. The STM32MP135F-DK's OpenSTLinux kernel has
//! `CONFIG_USB_PRINTER` unset, so there is no `/dev/usb/lp0` there; we write
//! ESC/POS straight to the printer's bulk-OUT endpoint instead.
//!
//! Selected with `PRINTER_PATH=usb` (first device with a USB printer-class
//! interface) or `PRINTER_PATH=usb:04b8:0e28` (hex vendor:product, for
//! printers that report a vendor-specific class). Access needs write
//! permission on `/dev/bus/usb/BBB/DDD` — see `kds/99-rusterando-printer.rules`.

use std::io::Write;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use escpos::driver::Driver;
use escpos::errors::PrinterError;
use nusb::descriptors::TransferType;
use nusb::transfer::{Bulk, Direction, Out};
use nusb::MaybeFuture;

/// USB interface class "Printer".
const USB_CLASS_PRINTER: u8 = 0x07;

/// A receipt is a few KB. A printer that hasn't drained it in this long is
/// off, jammed or out of paper — fail the print (it is retried) instead of
/// blocking the client loop forever on a stalled endpoint.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Which USB device to print to, parsed from `PRINTER_PATH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbSelector {
    pub vid_pid: Option<(u16, u16)>,
}

impl UsbSelector {
    /// `"usb"` → any printer-class device; `"usb:VVVV:PPPP"` → that device.
    /// `None` for anything else (a filesystem path).
    pub fn parse(spec: &str) -> Option<Self> {
        let rest = spec.strip_prefix("usb")?;
        if rest.is_empty() {
            return Some(Self { vid_pid: None });
        }
        let (vid, pid) = rest.strip_prefix(':')?.split_once(':')?;
        Some(Self {
            vid_pid: Some((
                u16::from_str_radix(vid, 16).ok()?,
                u16::from_str_radix(pid, 16).ok()?,
            )),
        })
    }
}

impl std::fmt::Display for UsbSelector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.vid_pid {
            Some((v, p)) => write!(f, "usb:{v:04x}:{p:04x}"),
            None => write!(f, "usb"),
        }
    }
}

/// Name of the first USB printer-class device plugged in ("TM-T20III", or
/// "04b8:0e28" when it reports no product string), without opening it —
/// for the display's live "printer connected?" indicator. Reads sysfs
/// only, a few milliseconds.
pub fn detect_printer() -> Option<String> {
    let devices = nusb::list_devices().wait().ok()?;
    devices
        .filter(|d| d.interfaces().any(|i| i.class() == USB_CLASS_PRINTER))
        .map(|d| match d.product_string() {
            Some(p) if !p.trim().is_empty() => p.trim().to_string(),
            _ => format!("{:04x}:{:04x}", d.vendor_id(), d.product_id()),
        })
        .next()
}

/// An open, claimed printer interface. Opened per receipt (like the file
/// driver) so a printer that was power-cycled or re-plugged — new device
/// address — is simply found again on the next print.
pub struct UsbDriver {
    name: String,
    writer: Mutex<nusb::io::EndpointWrite<Bulk>>,
}

impl UsbDriver {
    pub fn open(sel: UsbSelector) -> Result<Self> {
        let devices = nusb::list_devices().wait().context("list USB devices")?;
        let (info, interface_number) = devices
            .filter_map(|d| {
                let printer_class = d
                    .interfaces()
                    .find(|i| i.class() == USB_CLASS_PRINTER)
                    .map(|i| i.interface_number());
                let iface = match sel.vid_pid {
                    // Explicit device: trust it even with a vendor-specific
                    // class, falling back to its first interface.
                    Some((v, p)) if d.vendor_id() == v && d.product_id() == p => printer_class
                        .or_else(|| d.interfaces().next().map(|i| i.interface_number())),
                    Some(_) => None,
                    None => printer_class,
                }?;
                Some((d, iface))
            })
            .next()
            .ok_or_else(|| anyhow!("no USB printer found ({sel})"))?;

        let name = format!(
            "USB {:04x}:{:04x} {} (if {interface_number})",
            info.vendor_id(),
            info.product_id(),
            info.product_string().unwrap_or("printer"),
        );
        let device = info.open().wait().with_context(|| format!("open {name}"))?;
        // No usblp on the board, but on a Pi-style kernel it would hold the
        // interface — detach it so both setups work.
        let interface = device
            .detach_and_claim_interface(interface_number)
            .wait()
            .with_context(|| format!("claim {name}"))?;
        let out_addr = interface
            .descriptor()
            .and_then(|d| {
                d.endpoints()
                    .find(|e| {
                        e.transfer_type() == TransferType::Bulk && e.direction() == Direction::Out
                    })
                    .map(|e| e.address())
            })
            .ok_or_else(|| anyhow!("{name}: no bulk OUT endpoint"))?;
        let endpoint = interface
            .endpoint::<Bulk, Out>(out_addr)
            .with_context(|| format!("{name}: open endpoint {out_addr:#04x}"))?;
        Ok(Self {
            name,
            writer: Mutex::new(endpoint.writer(4096).with_write_timeout(WRITE_TIMEOUT)),
        })
    }
}

impl Driver for UsbDriver {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn write(&self, data: &[u8]) -> escpos::errors::Result<()> {
        let mut w = self.writer.lock().map_err(|e| PrinterError::Io(e.to_string()))?;
        w.write_all(data).map_err(|e| PrinterError::Io(e.to_string()))
    }

    fn read(&self, _buf: &mut [u8]) -> escpos::errors::Result<usize> {
        // We never query printer status.
        Ok(0)
    }

    fn flush(&self) -> escpos::errors::Result<()> {
        let mut w = self.writer.lock().map_err(|e| PrinterError::Io(e.to_string()))?;
        w.flush().map_err(|e| PrinterError::Io(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_selector() {
        assert_eq!(UsbSelector::parse("usb"), Some(UsbSelector { vid_pid: None }));
        assert_eq!(
            UsbSelector::parse("usb:04b8:0e28"),
            Some(UsbSelector {
                vid_pid: Some((0x04b8, 0x0e28))
            })
        );
        assert_eq!(UsbSelector::parse("/dev/usb/lp0"), None);
        assert_eq!(UsbSelector::parse("usb:zz:0e28"), None);
        assert_eq!(UsbSelector::parse("usbx"), None);
    }
}
