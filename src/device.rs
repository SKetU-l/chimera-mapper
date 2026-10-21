use hidapi::{HidApi, HidDevice};
use std::time::Duration;

use crate::config::AppResult;
use crate::hid::{PID_BLUETOOTH, PID_DONGLE, PID_WIRED, VID_KREO, format_report};
use crate::protocol::{self, FEATURE_LEN, OUTPUT_PAYLOAD};

const QUERY_SETTLE: Duration = Duration::from_millis(10);
const COMMAND_GAP: Duration = Duration::from_millis(30);
const PACKET_SETTLE: Duration = Duration::from_millis(50);

/// Gaps around each bulk packet. The 2.4G dongle relays packets over the
/// radio and silently drops them if they arrive faster than this.
#[derive(Clone, Copy)]
struct PacketTiming {
    after_header: Duration,
    after_packet: Duration,
}

const WIRED_TIMING: PacketTiming = PacketTiming {
    after_header: QUERY_SETTLE,
    after_packet: PACKET_SETTLE,
};

const WIRELESS_TIMING: PacketTiming = PacketTiming {
    after_header: Duration::from_millis(200),
    after_packet: Duration::from_millis(300),
};

/// The mouse's vendor configuration interface, used for reading settings.
pub struct Chimera {
    device: HidDevice,
    pub pid: u16,
    /// Only the Linux libusb write path needs the interface number.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    interface: i32,
    /// Bluetooth address for BT connections (hidapi reports it as the serial).
    address: Option<String>,
}

const BLUETOOTH_UNSUPPORTED: &str = "the mouse can't be configured over Bluetooth (its BT interface has no settings channel); \
     connect the cable or the 2.4G dongle once — settings are stored on the mouse and carry over to Bluetooth";

impl Chimera {
    pub fn open(api: &HidApi) -> AppResult<Self> {
        let rank = |pid: u16| match pid {
            PID_WIRED => 0,
            PID_DONGLE => 1,
            PID_BLUETOOTH => 2,
            _ => 3,
        };
        let info = api
            .device_list()
            .filter(|d| {
                d.vendor_id() == VID_KREO
                    && rank(d.product_id()) < 3
                    && d.usage_page() == protocol::VENDOR_USAGE_PAGE
            })
            .min_by_key(|d| rank(d.product_id()))
            .ok_or("Chimera configuration interface not found (is the mouse connected, and is the udev rule installed?)")?;
        // Right after a commit the mouse re-enumerates and the node exists
        // before udev has granted access to it, so give it a moment.
        let mut attempts = 0;
        let device = loop {
            match info.open_device(api) {
                Ok(device) => break device,
                Err(_) if attempts < 10 => {
                    attempts += 1;
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(e) => return Err(e.into()),
            }
        };
        Ok(Chimera {
            device,
            pid: info.product_id(),
            interface: info.interface_number(),
            address: info.serial_number().map(str::to_owned),
        })
    }

    pub fn is_wired(&self) -> bool {
        self.pid == PID_WIRED
    }

    pub fn connection(&self) -> &'static str {
        match self.pid {
            PID_WIRED => "wired",
            PID_DONGLE => "2.4G dongle",
            PID_BLUETOOTH => "bluetooth",
            _ => "unknown",
        }
    }

    pub fn is_bluetooth(&self) -> bool {
        self.pid == PID_BLUETOOTH
    }

    /// Battery level as reported by the standard Bluetooth battery service.
    pub fn bluetooth_battery(&self) -> Option<u8> {
        bluetooth_battery(self.address.as_deref()?)
    }

    /// Reads a setting. Returns the payload after the echoed query byte.
    pub fn query(&self, cmd: u8) -> AppResult<[u8; FEATURE_LEN - 2]> {
        if self.is_bluetooth() {
            return Err(BLUETOOTH_UNSUPPORTED.into());
        }
        let query = protocol::QUERY | cmd;
        self.device
            .send_feature_report(&protocol::feature(query, &[]))?;
        std::thread::sleep(QUERY_SETTLE);
        let mut buf = [0u8; FEATURE_LEN];
        buf[0] = protocol::REPORT_ID;
        self.device.get_feature_report(&mut buf)?;
        if buf[1] != query {
            return Err(format!("device did not answer query 0x{query:02x}").into());
        }
        let mut out = [0u8; FEATURE_LEN - 2];
        out.copy_from_slice(&buf[2..]);
        Ok(out)
    }

    /// Sends a command that takes effect immediately and is not persisted.
    pub fn set_live(&self, cmd: u8, payload: &[u8]) -> AppResult<()> {
        self.device
            .send_feature_report(&protocol::feature(cmd, payload))?;
        Ok(())
    }

    /// Opens a write session. Call `commit` to persist; dropping a session
    /// without committing leaves the mouse running its previous settings.
    pub fn session(self) -> AppResult<Session> {
        if self.is_bluetooth() {
            return Err(BLUETOOTH_UNSUPPORTED.into());
        }
        let wired = self.is_wired();
        let timing = if wired { WIRED_TIMING } else { WIRELESS_TIMING };
        #[cfg(target_os = "linux")]
        let link = {
            let link = usb::ControlLink::open(self.pid, self.interface)?;
            drop(self.device);
            Link::Usb(link)
        };
        #[cfg(not(target_os = "linux"))]
        let link = Link::Hid(self.device);
        Ok(Session {
            link,
            timing,
            wired,
        })
    }
}

/// Reads BlueZ's view of the Battery Service via `bluetoothctl`.
#[cfg(target_os = "linux")]
fn bluetooth_battery(address: &str) -> Option<u8> {
    let out = std::process::Command::new("bluetoothctl")
        .args(["info", address])
        .output()
        .ok()?;
    // "Battery Percentage: 0x5f (95)"
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.trim().strip_prefix("Battery Percentage:"))
        .and_then(|v| v.split('(').nth(1))
        .and_then(|v| v.trim_end_matches(')').trim().parse().ok())
}

#[cfg(not(target_os = "linux"))]
fn bluetooth_battery(_address: &str) -> Option<u8> {
    None
}

enum Link {
    DryRun,
    /// Platform HID stack, for platforms where output reports go out as
    /// SET_REPORT on the control pipe.
    #[cfg(not(target_os = "linux"))]
    Hid(HidDevice),
    /// Linux routes hidraw output reports to the interrupt OUT endpoint,
    /// which this firmware stalls, so writes go over the control pipe.
    #[cfg(target_os = "linux")]
    Usb(usb::ControlLink),
}

pub struct Session {
    link: Link,
    timing: PacketTiming,
    wired: bool,
}

impl Session {
    /// Prints frames instead of sending them; needs no mouse.
    pub fn dry_run() -> Session {
        Session {
            link: Link::DryRun,
            timing: WIRED_TIMING,
            wired: true,
        }
    }

    fn send_feature(&self, frame: &[u8; FEATURE_LEN]) -> AppResult<()> {
        match &self.link {
            Link::DryRun => println!("feature {}", format_report(frame)),
            #[cfg(not(target_os = "linux"))]
            Link::Hid(dev) => dev.send_feature_report(frame)?,
            #[cfg(target_os = "linux")]
            Link::Usb(link) => link.set_report(usb::FEATURE, frame)?,
        }
        Ok(())
    }

    fn send_output(&self, chunk: &[u8; OUTPUT_PAYLOAD]) -> AppResult<()> {
        let frame = protocol::output(chunk);
        match &self.link {
            Link::DryRun => println!("output  {}", format_report(&frame)),
            #[cfg(not(target_os = "linux"))]
            Link::Hid(dev) => {
                dev.send_output_report(&frame)?;
            }
            #[cfg(target_os = "linux")]
            Link::Usb(link) => link.set_report(usb::OUTPUT, &frame)?,
        }
        Ok(())
    }

    pub fn begin(&self) -> AppResult<()> {
        self.command(protocol::CMD_BEGIN, &[])
    }

    pub fn command(&self, cmd: u8, payload: &[u8]) -> AppResult<()> {
        self.send_feature(&protocol::feature(cmd, payload))?;
        std::thread::sleep(COMMAND_GAP);
        Ok(())
    }

    /// Persists to flash. A wired mouse reboots and re-enumerates, so the
    /// transfer itself usually reports a failure; the 2.4G dongle does not.
    pub fn commit(self) -> AppResult<()> {
        let result = self.send_feature(&protocol::feature(protocol::CMD_COMMIT, &[]));
        if self.wired {
            // The reboot cuts the transfer short; that is the expected outcome.
            return Ok(());
        }
        result.map_err(|e| format!("the mouse did not confirm saving the settings: {e}").into())
    }

    pub fn write_buttons(
        &self,
        bindings: &[protocol::Binding; protocol::BUTTON_COUNT],
    ) -> AppResult<()> {
        for (idx, packet) in protocol::encode_buttons(bindings).iter().enumerate() {
            self.send_feature(&protocol::buttons_header(idx as u8))?;
            std::thread::sleep(self.timing.after_header);
            self.send_output(packet)?;
            std::thread::sleep(self.timing.after_packet);
        }
        Ok(())
    }

    pub fn write_macro(&self, slot: u8, packets: &[[u8; OUTPUT_PAYLOAD]]) -> AppResult<()> {
        for (idx, packet) in packets.iter().enumerate() {
            self.send_feature(&protocol::macro_header(idx as u8, slot))?;
            std::thread::sleep(self.timing.after_header);
            self.send_output(packet)?;
            std::thread::sleep(self.timing.after_packet);
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod usb {
    use rusb::{DeviceHandle, GlobalContext};
    use std::time::Duration;

    use crate::config::AppResult;
    use crate::hid::VID_KREO;

    pub const OUTPUT: u8 = 0x02;
    pub const FEATURE: u8 = 0x03;
    const SET_REPORT: u8 = 0x09;
    const CLASS_INTERFACE_OUT: u8 = 0x21;
    const TIMEOUT: Duration = Duration::from_millis(500);

    pub struct ControlLink {
        handle: DeviceHandle<GlobalContext>,
        interface: u8,
        /// Set when the driver was detached by hand rather than by libusb.
        reattach: bool,
    }

    impl ControlLink {
        pub fn open(pid: u16, interface: i32) -> AppResult<Self> {
            let interface = u8::try_from(interface)
                .map_err(|_| "configuration interface number unknown for this connection")?;
            let handle = rusb::open_device_with_vid_pid(VID_KREO, pid).ok_or(
                "cannot open the mouse's USB device node (is the udev rule installed and the mouse replugged?)",
            )?;
            // With auto-detach, libusb detaches and claims in one step and
            // reattaches the kernel driver on release, so the hidraw node
            // comes back even if claiming fails.
            let auto = handle.set_auto_detach_kernel_driver(true).is_ok();
            let mut link = ControlLink {
                handle,
                interface,
                reattach: false,
            };
            if !auto && link.handle.kernel_driver_active(interface).unwrap_or(false) {
                link.handle.detach_kernel_driver(interface)?;
                link.reattach = true;
            }
            link.handle.claim_interface(interface)?;
            Ok(link)
        }

        pub fn set_report(&self, kind: u8, frame: &[u8]) -> AppResult<()> {
            let value = (kind as u16) << 8 | frame[0] as u16;
            self.handle.write_control(
                CLASS_INTERFACE_OUT,
                SET_REPORT,
                value,
                self.interface as u16,
                frame,
                TIMEOUT,
            )?;
            Ok(())
        }
    }

    impl Drop for ControlLink {
        // Hand the interface back to the kernel. This fails harmlessly if the
        // device rebooted after a commit (it re-enumerates with a fresh binding).
        fn drop(&mut self) {
            let _ = self.handle.release_interface(self.interface);
            if self.reattach {
                let _ = self.handle.attach_kernel_driver(self.interface);
            }
        }
    }
}
