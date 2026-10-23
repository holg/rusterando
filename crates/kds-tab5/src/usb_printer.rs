//! The receipt printer on the Tab5's USB-A port (USB host), plug and play.
//!
//! The board package starts ESP-IDF's USB host library (and switches the
//! port's 5 V on). A client of that library watches devices come and go: one
//! with a printer-class interface (07h) and a bulk-OUT endpoint is the
//! printer, its interface claimed while it's plugged in. Receipts (ESC/POS
//! bytes, see receipt-escpos) go out as bulk transfers. Everything runs on
//! one thread: the library calls our callbacks from inside
//! `usb_host_client_handle_events`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use esp_idf_svc::sys::{bsp, esp};

/// Bytes per bulk transfer.
const CHUNK: usize = 4096;
/// One chunk must be through in this time (the printer is busy printing).
const CHUNK_TIMEOUT: Duration = Duration::from_secs(15);
/// Epson's USB vendor id, for a readable name.
const VID_EPSON: u16 = 0x04b8;

/// Device events from the library's callback, handled on the USB thread.
enum UsbEvent {
    New(u8),
    /// The device handle (as usize: raw pointers aren't Send).
    Gone(usize),
}

static EVENTS: Mutex<Vec<UsbEvent>> = Mutex::new(Vec::new());
static TRANSFER_DONE: AtomicBool = AtomicBool::new(false);

struct Job {
    bytes: Vec<u8>,
    done: SyncSender<Result<()>>,
}

/// Handle to the USB thread.
#[derive(Clone)]
pub struct UsbPrinter {
    jobs: Sender<Job>,
    present: Arc<AtomicBool>,
}

impl UsbPrinter {
    /// Start the USB host and the printer thread. `on_change` gets the
    /// printer's name when one is plugged in and `None` when it's gone.
    pub fn start(on_change: impl Fn(Option<String>) + Send + 'static) -> Result<Self> {
        let (jobs, rx) = mpsc::channel();
        let present = Arc::new(AtomicBool::new(false));
        let flag = present.clone();
        std::thread::Builder::new()
            .name("usb".into())
            .stack_size(8 * 1024)
            .spawn(move || {
                if let Err(e) = run(rx, flag, on_change) {
                    log::error!("usb printer: {e:#}");
                }
            })?;
        Ok(Self { jobs, present })
    }

    pub fn present(&self) -> bool {
        self.present.load(Ordering::Relaxed)
    }

    /// Send `bytes` to the printer; returns once they're through (or failed).
    pub fn print(&self, bytes: Vec<u8>) -> Result<()> {
        let (done, result) = mpsc::sync_channel(1);
        self.jobs
            .send(Job { bytes, done })
            .map_err(|_| anyhow!("USB thread not running"))?;
        result.recv().map_err(|_| anyhow!("USB thread ended"))?
    }
}

struct Printer {
    dev: bsp::usb_device_handle_t,
    interface: u8,
    ep_out: u8,
    name: String,
}

fn run(
    jobs: Receiver<Job>,
    present: Arc<AtomicBool>,
    on_change: impl Fn(Option<String>),
) -> Result<()> {
    // SAFETY: board-package and USB host library calls, all on this thread.
    let client = unsafe {
        esp!(bsp::bsp_usb_host_start(
            bsp::bsp_usb_host_power_mode_t_BSP_USB_HOST_POWER_MODE_USB_DEV,
            true
        ))?;
        let mut cfg: bsp::usb_host_client_config_t = core::mem::zeroed();
        cfg.is_synchronous = false;
        cfg.max_num_event_msg = 5;
        cfg.__bindgen_anon_1.async_.client_event_callback = Some(client_event);
        cfg.__bindgen_anon_1.async_.callback_arg = core::ptr::null_mut();
        let mut client = core::ptr::null_mut();
        esp!(bsp::usb_host_client_register(&cfg, &mut client))?;
        client
    };
    log::info!("usb printer: host up, waiting for a printer");

    let mut printer: Option<Printer> = None;
    loop {
        // SAFETY: live client handle; callbacks run in here.
        unsafe { bsp::usb_host_client_handle_events(client, ticks(Duration::from_millis(50))) };

        let events = std::mem::take(&mut *EVENTS.lock().unwrap());
        for event in events {
            match event {
                UsbEvent::New(address) if printer.is_none() => match open_printer(client, address) {
                    Ok(p) => {
                        log::info!("usb printer: {} plugged in", p.name);
                        present.store(true, Ordering::Relaxed);
                        on_change(Some(p.name.clone()));
                        printer = Some(p);
                    }
                    Err(e) => log::info!("usb printer: device {address} is not a printer: {e:#}"),
                },
                UsbEvent::New(_) => {}
                UsbEvent::Gone(dev) => {
                    if printer.as_ref().is_some_and(|p| p.dev as usize == dev) {
                        let p = printer.take().unwrap();
                        log::info!("usb printer: {} unplugged", p.name);
                        close_printer(client, &p);
                        present.store(false, Ordering::Relaxed);
                        on_change(None);
                    }
                }
            }
        }

        while let Ok(job) = jobs.try_recv() {
            let result = match &printer {
                Some(p) => write(client, p, &job.bytes),
                None => Err(anyhow!("kein Drucker angeschlossen")),
            };
            let _ = job.done.send(result);
        }
    }
}

/// Library callback: new device / device gone.
unsafe extern "C" fn client_event(msg: *const bsp::usb_host_client_event_msg_t, _arg: *mut core::ffi::c_void) {
    let msg = &*msg;
    let event = match msg.event {
        bsp::usb_host_client_event_t_USB_HOST_CLIENT_EVENT_NEW_DEV => {
            UsbEvent::New(msg.__bindgen_anon_1.new_dev.address)
        }
        bsp::usb_host_client_event_t_USB_HOST_CLIENT_EVENT_DEV_GONE => {
            UsbEvent::Gone(msg.__bindgen_anon_1.dev_gone.dev_hdl as usize)
        }
        _ => return,
    };
    if let Ok(mut events) = EVENTS.lock() {
        events.push(event);
    }
}

unsafe extern "C" fn transfer_done(_transfer: *mut bsp::usb_transfer_t) {
    TRANSFER_DONE.store(true, Ordering::Release);
}

/// Open the device at `address` and claim its printer interface.
fn open_printer(client: bsp::usb_host_client_handle_t, address: u8) -> Result<Printer> {
    let mut dev = core::ptr::null_mut();
    // SAFETY: descriptors are owned by the library and live while the
    // device is open; everything is read before returning.
    unsafe {
        esp!(bsp::usb_host_device_open(client, address, &mut dev))?;
        match find_printer_interface(client, dev) {
            Ok(p) => Ok(p),
            Err(e) => {
                bsp::usb_host_device_close(client, dev);
                Err(e)
            }
        }
    }
}

unsafe fn find_printer_interface(
    client: bsp::usb_host_client_handle_t,
    dev: bsp::usb_device_handle_t,
) -> Result<Printer> {
    let mut device: *const bsp::usb_device_desc_t = core::ptr::null();
    esp!(bsp::usb_host_get_device_descriptor(dev, &mut device))?;
    let mut config: *const bsp::usb_config_desc_t = core::ptr::null();
    esp!(bsp::usb_host_get_active_config_descriptor(dev, &mut config))?;
    let (vid, pid) = ((*device).idVendor, (*device).idProduct);
    let total = (*config).wTotalLength;

    for number in 0..(*config).bNumInterfaces {
        let mut offset = 0;
        let intf = bsp::usb_parse_interface_descriptor(config, number, 0, &mut offset);
        if intf.is_null() || (*intf).bInterfaceClass as u32 != bsp::USB_CLASS_PRINTER {
            continue;
        }
        for index in 0..(*intf).bNumEndpoints as i32 {
            let mut ep_offset = offset;
            let ep = bsp::usb_parse_endpoint_descriptor_by_index(intf, index, total, &mut ep_offset);
            if ep.is_null() {
                continue;
            }
            let bulk = ((*ep).bmAttributes as u32 & 0x03) == bsp::USB_BM_ATTRIBUTES_XFER_BULK;
            let out = ((*ep).bEndpointAddress as u32 & bsp::USB_B_ENDPOINT_ADDRESS_EP_DIR_MASK) == 0;
            if bulk && out {
                let interface = (*intf).bInterfaceNumber;
                esp!(bsp::usb_host_interface_claim(client, dev, interface, (*intf).bAlternateSetting))?;
                let maker = if vid == VID_EPSON { "Epson " } else { "" };
                return Ok(Printer {
                    dev,
                    interface,
                    ep_out: (*ep).bEndpointAddress,
                    name: format!("{maker}{vid:04x}:{pid:04x}"),
                });
            }
        }
    }
    bail!("no printer interface with a bulk-OUT endpoint ({vid:04x}:{pid:04x})")
}

fn close_printer(client: bsp::usb_host_client_handle_t, p: &Printer) {
    // SAFETY: the handles came from open_printer.
    unsafe {
        bsp::usb_host_interface_release(client, p.dev, p.interface);
        bsp::usb_host_device_close(client, p.dev);
    }
}

/// Send `bytes` as bulk-OUT transfers of up to [`CHUNK`] bytes each.
fn write(client: bsp::usb_host_client_handle_t, p: &Printer, bytes: &[u8]) -> Result<()> {
    let mut transfer: *mut bsp::usb_transfer_t = core::ptr::null_mut();
    // SAFETY: the transfer is ours until submitted and again once its
    // callback ran; on a timeout it is leaked rather than freed while in flight.
    unsafe {
        esp!(bsp::usb_host_transfer_alloc(CHUNK, 0, &mut transfer))?;
        for chunk in bytes.chunks(CHUNK) {
            let t = &mut *transfer;
            core::ptr::copy_nonoverlapping(chunk.as_ptr(), t.data_buffer, chunk.len());
            t.num_bytes = chunk.len() as i32;
            t.device_handle = p.dev;
            t.bEndpointAddress = p.ep_out;
            t.callback = Some(transfer_done);
            t.context = core::ptr::null_mut();
            TRANSFER_DONE.store(false, Ordering::Release);
            if let Err(e) = esp!(bsp::usb_host_transfer_submit(transfer)) {
                bsp::usb_host_transfer_free(transfer);
                return Err(e.into());
            }
            let started = Instant::now();
            while !TRANSFER_DONE.load(Ordering::Acquire) {
                if started.elapsed() > CHUNK_TIMEOUT {
                    bail!("printer did not take the data (timeout)");
                }
                bsp::usb_host_client_handle_events(client, ticks(Duration::from_millis(10)));
            }
            if t.status != bsp::usb_transfer_status_t_USB_TRANSFER_STATUS_COMPLETED {
                let status = t.status;
                bsp::usb_host_transfer_free(transfer);
                bail!("USB transfer failed (status {status})");
            }
        }
        bsp::usb_host_transfer_free(transfer);
    }
    Ok(())
}

fn ticks(d: Duration) -> u32 {
    // CONFIG_FREERTOS_HZ=1000: one tick per millisecond.
    d.as_millis().max(1) as u32
}
