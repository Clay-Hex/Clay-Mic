use crate::protocol::atvv::{self, AtvvCapabilities, ATVV_CONTROL, ATVV_RX_AUDIO, ATVV_SERVICE, ATVV_TX, GET_CAPS};

const DIS_SERVICE: &str = "0000180a-0000-1000-8000-00805f9b34fb";
const PNP_ID_CHAR: &str = "00002a50-0000-1000-8000-00805f9b34fb";
const MODEL_NUMBER_CHAR: &str = "00002a24-0000-1000-8000-00805f9b34fb";

pub struct BleDevice {
    pub name: String,
    pub address: String,
    pub caps: AtvvCapabilities,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BleDeviceInfo {
    pub id: String,
    pub name: String,
}

#[cfg(target_os = "windows")]
pub mod platform {
    use super::*;
    use std::future::IntoFuture;
    use windows::core::HSTRING;
    use windows::Devices::Bluetooth::GenericAttributeProfile::*;
    use windows::Devices::Bluetooth::{BluetoothCacheMode, BluetoothLEDevice};
    use windows::Devices::Enumeration::DeviceInformation;
    use windows::Foundation::TypedEventHandler;
    use windows::Storage::Streams::{DataReader, DataWriter};
    use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

    /// RAII guard for WinRT apartment initialization
    struct WinRtApartment;
    impl Drop for WinRtApartment {
        fn drop(&mut self) { unsafe { RoUninitialize() }; }
    }

    pub struct WinRtBleDevice {
        pub device: BluetoothLEDevice,
        pub name: String,
        pub address: String,
        pub tx_char: GattCharacteristic,
        pub caps: AtvvCapabilities,
        pub vendor_id: Option<u16>,
        pub product_id: Option<u16>,
        pub model: Option<String>,
        /// Handed to the voice session listener after connect.
        pub audio_rx: Option<std::sync::mpsc::Receiver<Vec<u8>>>,
        pub control_rx: Option<std::sync::mpsc::Receiver<Vec<u8>>>,
        /// Keeping the characteristic objects alive keeps notifications flowing.
        _rx_char: GattCharacteristic,
        _control_char: GattCharacteristic,
        /// Must keep these alive or notifications stop
        _audio_token: i64,
        _control_token: i64,
    }

    fn block_on<T, O>(op: windows::core::Result<O>) -> Result<T, String>
    where O: IntoFuture<Output = windows::core::Result<T>> {
        let fut = op.map_err(|e| e.to_string())?;
        futures::executor::block_on(fut.into_future()).map_err(|e| e.to_string())
    }

    fn guid_from_hex(s: &str) -> windows::core::GUID {
        windows::core::GUID::from_u128(u128::from_str_radix(&s.replace('-', ""), 16).unwrap_or(0))
    }

    fn buffer_to_vec(buf: &windows::Storage::Streams::IBuffer) -> Result<Vec<u8>, String> {
        let len = buf.Length().map_err(|e| e.to_string())? as usize;
        let reader = DataReader::FromBuffer(buf).map_err(|e| e.to_string())?;
        let mut bytes = vec![0u8; len];
        reader.ReadBytes(&mut bytes).map_err(|e| e.to_string())?;
        let _ = reader.Close();
        Ok(bytes)
    }

    fn gatt_write(ch: &GattCharacteristic, data: &[u8]) -> Result<(), String> {
        let w = DataWriter::new().map_err(|e| e.to_string())?;
        w.WriteBytes(data).map_err(|e| e.to_string())?;
        let buf = w.DetachBuffer().map_err(|e| e.to_string())?;
        let status = block_on(ch.WriteValueAsync(&buf))?;
        if status == GattCommunicationStatus::Success { Ok(()) } else { Err(format!("GATT write: {:?}", status)) }
    }

    /// GATT discovery can fail transiently right after a connect, so retry with
    /// a short backoff and fall back to the cached view before giving up.
    fn find_service(dev: &BluetoothLEDevice, uuid: windows::core::GUID) -> Result<GattDeviceService, String> {
        let mut last = String::new();
        for attempt in 0..4u64 {
            let mode = if attempt < 2 { BluetoothCacheMode::Uncached } else { BluetoothCacheMode::Cached };
            let found = (|| -> Result<GattDeviceService, String> {
                let result = block_on(dev.GetGattServicesForUuidWithCacheModeAsync(uuid, mode))?;
                if result.Status().map_err(|e| e.to_string())? != GattCommunicationStatus::Success {
                    return Err("Service not found".into());
                }
                let svcs = result.Services().map_err(|e| e.to_string())?;
                if svcs.Size().map_err(|e| e.to_string())? == 0 { return Err("Service not found".into()); }
                svcs.GetAt(0).map_err(|e| e.to_string())
            })();
            match found {
                Ok(service) => return Ok(service),
                Err(error) => {
                    last = error;
                    if attempt < 3 {
                        std::thread::sleep(std::time::Duration::from_millis(250 * (attempt + 1)));
                    }
                }
            }
        }
        Err(format!("Service not found（重试后仍失败：{last}）"))
    }

    /// Same retry/fallback strategy as [`find_service`].
    fn find_char(svc: &GattDeviceService, uuid: windows::core::GUID) -> Result<GattCharacteristic, String> {
        let mut last = String::new();
        for attempt in 0..4u64 {
            let mode = if attempt < 2 { BluetoothCacheMode::Uncached } else { BluetoothCacheMode::Cached };
            let found = (|| -> Result<GattCharacteristic, String> {
                let result = block_on(svc.GetCharacteristicsForUuidWithCacheModeAsync(uuid, mode))?;
                if result.Status().map_err(|e| e.to_string())? != GattCommunicationStatus::Success {
                    return Err("Char not found".into());
                }
                let cs = result.Characteristics().map_err(|e| e.to_string())?;
                if cs.Size().map_err(|e| e.to_string())? == 0 { return Err("Char not found".into()); }
                cs.GetAt(0).map_err(|e| e.to_string())
            })();
            match found {
                Ok(characteristic) => return Ok(characteristic),
                Err(error) => {
                    last = error;
                    if attempt < 3 {
                        std::thread::sleep(std::time::Duration::from_millis(250 * (attempt + 1)));
                    }
                }
            }
        }
        Err(format!("Char not found（重试后仍失败：{last}）"))
    }

    /// Single-attempt service lookup, used where a retry budget would be
    /// wasted (probing candidates, optional services).
    fn service_once(dev: &BluetoothLEDevice, uuid: &str) -> Result<GattDeviceService, String> {
        let result = block_on(dev.GetGattServicesForUuidWithCacheModeAsync(
            guid_from_hex(uuid),
            BluetoothCacheMode::Uncached,
        ))?;
        if result.Status().map_err(|e| e.to_string())? != GattCommunicationStatus::Success {
            return Err("Service not found".into());
        }
        let services = result.Services().map_err(|e| e.to_string())?;
        if services.Size().map_err(|e| e.to_string())? == 0 {
            return Err("Service not found".into());
        }
        services.GetAt(0).map_err(|e| e.to_string())
    }

    fn char_once(svc: &GattDeviceService, uuid: &str) -> Result<GattCharacteristic, String> {
        let result = block_on(svc.GetCharacteristicsForUuidWithCacheModeAsync(
            guid_from_hex(uuid),
            BluetoothCacheMode::Uncached,
        ))?;
        if result.Status().map_err(|e| e.to_string())? != GattCommunicationStatus::Success {
            return Err("Char not found".into());
        }
        let chars = result.Characteristics().map_err(|e| e.to_string())?;
        if chars.Size().map_err(|e| e.to_string())? == 0 {
            return Err("Char not found".into());
        }
        chars.GetAt(0).map_err(|e| e.to_string())
    }

    fn read_dis_bytes(service: &GattDeviceService, uuid: &str) -> Option<Vec<u8>> {
        let characteristic = char_once(service, uuid).ok()?;
        let result = block_on(
            characteristic.ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached),
        )
        .ok()?;
        if result.Status().ok()? != GattCommunicationStatus::Success {
            return None;
        }
        buffer_to_vec(&result.Value().ok()?).ok()
    }

    /// PnP ID layout is little-endian:
    /// `[source u8, vendor u16, product u16, version u16]`.
    fn read_pnp_id(service: &GattDeviceService) -> Option<(u16, u16)> {
        let bytes = read_dis_bytes(service, PNP_ID_CHAR)?;
        if bytes.len() < 5 {
            return None;
        }
        Some((
            u16::from_le_bytes([bytes[1], bytes[2]]),
            u16::from_le_bytes([bytes[3], bytes[4]]),
        ))
    }

    fn read_model_number(service: &GattDeviceService) -> Option<String> {
        let bytes = read_dis_bytes(service, MODEL_NUMBER_CHAR)?;
        let text = String::from_utf8_lossy(&bytes)
            .trim_matches(char::from(0))
            .trim()
            .to_string();
        (!text.is_empty()).then_some(text)
    }

    #[derive(Default)]
    struct DeviceIdentity {
        vendor_id: Option<u16>,
        product_id: Option<u16>,
        model: Option<String>,
    }

    fn read_device_identity(device: &BluetoothLEDevice) -> DeviceIdentity {
        let Ok(service) = service_once(device, DIS_SERVICE) else {
            log::warn!("device exposes no Device Information Service (0x180A)");
            return DeviceIdentity::default();
        };
        let (vendor_id, product_id) = match read_pnp_id(&service) {
            Some((vendor, product)) => (Some(vendor), Some(product)),
            None => {
                log::warn!("no PnP ID (0x2A50); button suppression cannot bind to this device");
                (None, None)
            }
        };
        DeviceIdentity {
            vendor_id,
            product_id,
            model: read_model_number(&service),
        }
    }

    /// Subscribe to GATT notifications. Returns the EventRegistrationToken
    /// which MUST be kept alive for notifications to continue.
    fn subscribe(ch: &GattCharacteristic, sender: std::sync::mpsc::Sender<Vec<u8>>) -> Result<i64, String> {
        let handler = TypedEventHandler::<GattCharacteristic, GattValueChangedEventArgs>::new(
            move |_, args| {
                let result = args
                    .ok()
                    .ok()
                    .and_then(|args| args.CharacteristicValue().ok())
                    .and_then(|buffer| buffer_to_vec(&buffer).ok());
                if let Some(data) = result {
                    let _ = sender.send(data);
                }
                Ok(())
            },
        );
        // Store the token - dropping it unsubscribes!
        let token = ch.ValueChanged(&handler).map_err(|e| e.to_string())?;

        // Enable notifications
        let props = ch.CharacteristicProperties().map_err(|e| e.to_string())?;
        let dv = if props.0 & GattCharacteristicProperties::Notify.0 != 0 {
            GattClientCharacteristicConfigurationDescriptorValue::Notify
        } else if props.0 & GattCharacteristicProperties::Indicate.0 != 0 {
            GattClientCharacteristicConfigurationDescriptorValue::Indicate
        } else { return Err("No Notify/Indicate".into()); };

        let status = block_on(ch.WriteClientCharacteristicConfigurationDescriptorAsync(dv))?;
        if status == GattCommunicationStatus::Success {
            Ok(token)  // Return token as i64 for storage
        } else {
            Err(format!("Subscribe: {:?}", status))
        }
    }

    /// Connect to a paired BLE device by its Windows device ID.
    /// The device_id is like: "BTHLE\\DEV_14BEFCF3D837\\..."
    pub fn connect_by_id(device_id: &str) -> Result<WinRtBleDevice, String> {
        // Initialize WinRT apartment on this thread
        unsafe { RoInitialize(RO_INIT_MULTITHREADED).map_err(|e| format!("RoInitialize: {}", e))? };
        let _apartment = WinRtApartment;

        log::info!("Connecting to device: {}", device_id);

        // Get device by Windows device ID
        let device = block_on(BluetoothLEDevice::FromIdAsync(&HSTRING::from(device_id)))?;
        let name = device.Name().map(|n| n.to_string()).unwrap_or_else(|_| "Unknown".to_string());
        log::info!("Got device: {}", name);

        // Discover ATVV service and characteristics
        let atvv = find_service(&device, guid_from_hex(ATVV_SERVICE))?;
        let tx = find_char(&atvv, guid_from_hex(ATVV_TX))?;
        let rx = find_char(&atvv, guid_from_hex(ATVV_RX_AUDIO))?;
        let ctrl = find_char(&atvv, guid_from_hex(ATVV_CONTROL))?;
        log::info!("Found ATVV chars");

        // Send GET_CAPS
        gatt_write(&tx, &GET_CAPS)?;
        log::info!("Sent GET_CAPS");

        // Subscribe to control and wait for capabilities
        let (cap_tx, control_rx) = std::sync::mpsc::channel();
        let _control_token = subscribe(&ctrl, cap_tx)?;
        let caps_data = control_rx.recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| "Caps timeout".to_string())?;
        let caps = atvv::parse_caps_response(&caps_data).ok_or("Bad caps")?;
        log::info!("ATVV: {:?}", caps);

        // Subscribe to audio notifications (keep token alive!)
        let (audio_tx, audio_rx) = std::sync::mpsc::channel();
        let _audio_token = subscribe(&rx, audio_tx)?;

        // Keep the apartment alive by leaking it into the device struct
        // The WinRtApartment will be dropped when the device is dropped
        std::mem::forget(_apartment);

        let identity = read_device_identity(&device);

        Ok(WinRtBleDevice {
            device, name,
            address: device_id.to_string(),
            tx_char: tx, caps,
            vendor_id: identity.vendor_id,
            product_id: identity.product_id,
            model: identity.model,
            audio_rx: Some(audio_rx), control_rx: Some(control_rx),
            _rx_char: rx, _control_char: ctrl,
            _audio_token, _control_token,
        })
    }

    /// Connect by Bluetooth address (MAC). Calls FromBluetoothAddressAsync.
    pub fn connect_by_address(address: &str) -> Result<WinRtBleDevice, String> {
        // Initialize WinRT apartment on this thread
        unsafe { RoInitialize(RO_INIT_MULTITHREADED).map_err(|e| format!("RoInitialize: {}", e))? };
        let _apartment = WinRtApartment;

        let addr_str = address.replace(':', "");
        let addr_val = u64::from_str_radix(&addr_str, 16).map_err(|e| format!("Bad address: {}", e))?;
        log::info!("Connecting to {} (0x{:X})...", address, addr_val);

        let device = block_on(BluetoothLEDevice::FromBluetoothAddressAsync(addr_val))?;
        let name = device.Name().map(|n| n.to_string()).unwrap_or_else(|_| "Unknown".to_string());
        log::info!("Got device: {}", name);

        let atvv = find_service(&device, guid_from_hex(ATVV_SERVICE))?;
        let tx = find_char(&atvv, guid_from_hex(ATVV_TX))?;
        let rx = find_char(&atvv, guid_from_hex(ATVV_RX_AUDIO))?;
        let ctrl = find_char(&atvv, guid_from_hex(ATVV_CONTROL))?;
        log::info!("Found ATVV chars");

        gatt_write(&tx, &GET_CAPS)?;
        log::info!("Sent GET_CAPS");

        let (cap_tx, control_rx) = std::sync::mpsc::channel();
        let _control_token = subscribe(&ctrl, cap_tx)?;
        let caps_data = control_rx.recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| "Caps timeout".to_string())?;
        let caps = atvv::parse_caps_response(&caps_data).ok_or("Bad caps")?;
        log::info!("ATVV: {:?}", caps);

        let (audio_tx, audio_rx) = std::sync::mpsc::channel();
        let _audio_token = subscribe(&rx, audio_tx)?;

        // Keep apartment alive
        std::mem::forget(_apartment);

        let identity = read_device_identity(&device);

        Ok(WinRtBleDevice {
            device, name,
            address: address.to_string(),
            tx_char: tx, caps,
            vendor_id: identity.vendor_id,
            product_id: identity.product_id,
            model: identity.model,
            audio_rx: Some(audio_rx), control_rx: Some(control_rx),
            _rx_char: rx, _control_char: ctrl,
            _audio_token, _control_token,
        })
    }

    pub fn send_mic_open(d: &WinRtBleDevice) -> Result<(), String> {
        gatt_write(&d.tx_char, &atvv::mic_open_message(d.caps.version))
    }
    pub fn send_mic_close(d: &WinRtBleDevice, sid: u8) -> Result<(), String> {
        gatt_write(&d.tx_char, &atvv::mic_close_message(d.caps.version, sid))
    }

    /// Enumerate paired BLE devices that expose the ATVV voice service.
    ///
    /// ATVV capability is not AQS-filterable, so every paired candidate is
    /// probed over GATT and a refresh costs one round-trip per candidate.
    pub fn list_paired_ble_devices() -> Result<Vec<BleDeviceInfo>, String> {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED).map_err(|e| format!("RoInitialize: {}", e))? };
        let _apartment = WinRtApartment;

        let selector = BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)
            .map_err(|e| e.to_string())?;
        let collection = block_on(DeviceInformation::FindAllAsyncAqsFilter(&selector))?;

        let count = collection.Size().map_err(|e| e.to_string())?;
        let mut devices = Vec::new();
        for index in 0..count {
            let info = match collection.GetAt(index) {
                Ok(info) => info,
                Err(_) => continue,
            };
            let id = match info.Id() {
                Ok(id) => id.to_string(),
                Err(_) => continue,
            };
            let device = match block_on(BluetoothLEDevice::FromIdAsync(&HSTRING::from(id.as_str()))) {
                Ok(device) => device,
                Err(error) => {
                    log::debug!("probe: {id} not reachable ({error})");
                    continue;
                }
            };
            if service_once(&device, ATVV_SERVICE).is_err() {
                continue;
            }
            let raw_name = info.Name().map(|n| n.to_string()).unwrap_or_default();
            let name = if raw_name.trim().is_empty() {
                "未知设备".to_string()
            } else {
                raw_name
            };
            devices.push(BleDeviceInfo { id, name });
        }

        log::info!("Found {} ATVV-capable paired BLE device(s)", devices.len());
        Ok(devices)
    }
}

#[cfg(not(target_os = "windows"))]
pub mod platform {
    use super::*;
    pub fn connect_by_id(_id: &str) -> Result<BleDevice, String> { Err("Not supported".into()) }
    pub fn connect_by_address(_a: &str) -> Result<BleDevice, String> { Err("Not supported".into()) }
}

/// True only for a plain Bluetooth MAC like `37:D8:FC:EF:EB:14`.
/// Windows BLE device ids (e.g. `BluetoothLE#BluetoothLE37:D8:...-...`)
/// also contain colons, so a naive `contains(':')` check is wrong.
fn is_mac_address(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 17 {
        return false;
    }
    for (i, b) in bytes.iter().enumerate() {
        if i % 3 == 2 {
            if *b != b':' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// List BLE devices already paired with the OS. Windows uses the WinRT
/// pairing-state selector; other platforms return an empty list.
pub fn list_paired_devices() -> Result<Vec<BleDeviceInfo>, String> {
    #[cfg(target_os = "windows")]
    { platform::list_paired_ble_devices() }
    #[cfg(not(target_os = "windows"))]
    { Ok(Vec::new()) }
}

// ---------------------------------------------------------------------------
// Persistent session (keeps the GATT connection + subscriptions alive)
// ---------------------------------------------------------------------------

/// The connected remote, kept alive for the process so GATT notifications
/// (ATVV audio) keep flowing between key presses.
#[cfg(target_os = "windows")]
static SESSION: std::sync::Mutex<Option<platform::WinRtBleDevice>> =
    std::sync::Mutex::new(None);

/// Connect to the remote, keep the session alive, and hand the audio/control
/// channels to the voice session listener.
pub fn connect_and_store(address: &str) -> Result<BleDevice, String> {
    #[cfg(target_os = "windows")]
    {
        // Release any previous connection first so its GATT link cannot
        // interfere with the new service/characteristic discovery.
        *SESSION.lock().unwrap() = None;

        let mut device = if is_mac_address(address) {
            platform::connect_by_address(address)?
        } else {
            platform::connect_by_id(address)?
        };
        let info = BleDevice {
            name: device.name.clone(),
            address: device.address.clone(),
            caps: device.caps.clone(),
            vendor_id: device.vendor_id,
            product_id: device.product_id,
            model: device.model.clone(),
        };
        log::info!(
            "device identity: vendor={:?} product={:?} model={:?}",
            info.vendor_id,
            info.product_id,
            info.model
        );
        let caps = device.caps.clone();
        let audio_rx = device.audio_rx.take();
        let control_rx = device.control_rx.take();
        *SESSION.lock().unwrap() = Some(device);

        if let (Some(audio), Some(control)) = (audio_rx, control_rx) {
            crate::voice::start_session(audio, control, caps);
        }
        Ok(info)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = address;
        Err("Not supported".into())
    }
}

/// Whether a remote session is currently held.
pub fn is_connected() -> bool {
    #[cfg(target_os = "windows")]
    {
        SESSION.lock().unwrap().is_some()
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Drop the session (releases the GATT connection).
pub fn disconnect() {
    #[cfg(target_os = "windows")]
    {
        *SESSION.lock().unwrap() = None;
    }
}

/// Reply to the remote's microphone request by opening the ATVV microphone.
pub fn send_mic_open() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let guard = SESSION.lock().unwrap();
        let device = guard.as_ref().ok_or_else(|| "未连接遥控器".to_string())?;
        platform::send_mic_open(device)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Not supported".into())
    }
}

/// Close the ATVV microphone for `stream_id`.
pub fn send_mic_close(stream_id: u8) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let guard = SESSION.lock().unwrap();
        let device = guard.as_ref().ok_or_else(|| "未连接遥控器".to_string())?;
        platform::send_mic_close(device, stream_id)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = stream_id;
        Err("Not supported".into())
    }
}
