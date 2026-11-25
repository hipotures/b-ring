//! `bring pair --ble`: the Ring as a Bluetooth LE peripheral, through BlueZ
//! on the system D-Bus (pure Rust `zbus`, no libdbus).
//!
//! Exports a one-characteristic GATT application plus an advertisement, and
//! registers both with the first powered adapter. No bonding and no OS-level
//! Bluetooth pairing: the Moon connects, reads 31 bytes, disconnects
//! (`bring_core::pairing::ble`). Dropping [`BleBeacon`] unregisters both.

use std::collections::HashMap;

use bring_core::pairing::ble::{LOCAL_NAME, PAYLOAD_CHAR_UUID, SERVICE_UUID};
use bring_core::pairing::payload::PAYLOAD_LEN;
use zbus::fdo::{self, ObjectManager};
use zbus::{Connection, interface};
use zbus::{ObjectPath, OwnedObjectPath, OwnedVariant as OwnedValue};

const BLUEZ: &str = "org.bluez";
pub const APP_PATH: &str = "/io/github/hipotures/bring/pair";
pub const SERVICE_PATH: &str = "/io/github/hipotures/bring/pair/service0";
pub const CHAR_PATH: &str = "/io/github/hipotures/bring/pair/service0/char0";
pub const ADV_PATH: &str = "/io/github/hipotures/bring/adv0";

struct GattService;

#[interface(name = "org.bluez.GattService1")]
impl GattService {
    #[zbus(property(emits_changed_signal = "const"), name = "UUID")]
    fn uuid(&self) -> String {
        SERVICE_UUID.into()
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn primary(&self) -> bool {
        true
    }
}

struct PayloadCharacteristic {
    value: [u8; PAYLOAD_LEN],
}

#[interface(name = "org.bluez.GattCharacteristic1")]
impl PayloadCharacteristic {
    /// BlueZ passes `offset` for long reads when the ATT MTU is small.
    fn read_value(&self, options: HashMap<String, OwnedValue>) -> fdo::Result<Vec<u8>> {
        let offset = options
            .get("offset")
            .and_then(|v| u16::try_from(v).ok())
            .unwrap_or(0) as usize;
        self.value
            .get(offset..)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| fdo::Error::InvalidArgs("offset past the end".into()))
    }

    #[zbus(property(emits_changed_signal = "const"), name = "UUID")]
    fn uuid(&self) -> String {
        PAYLOAD_CHAR_UUID.into()
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn service(&self) -> OwnedObjectPath {
        ObjectPath::from_static_str_unchecked(SERVICE_PATH).into()
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn flags(&self) -> Vec<String> {
        vec!["read".into()]
    }
}

struct Advertisement;

#[interface(name = "org.bluez.LEAdvertisement1")]
impl Advertisement {
    /// Called by BlueZ when it drops the advertisement on its own.
    fn release(&self) {}

    #[zbus(property(emits_changed_signal = "const"), name = "Type")]
    fn kind(&self) -> String {
        "peripheral".into()
    }

    #[zbus(property(emits_changed_signal = "const"), name = "ServiceUUIDs")]
    fn service_uuids(&self) -> Vec<String> {
        vec![SERVICE_UUID.into()]
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn local_name(&self) -> String {
        LOCAL_NAME.into()
    }
}

/// Exports the GATT application and the advertisement on `conn`.
pub async fn export(conn: &Connection, payload: [u8; PAYLOAD_LEN]) -> zbus::Result<()> {
    let server = conn.object_server();
    server.at(SERVICE_PATH, GattService).await?;
    server
        .at(CHAR_PATH, PayloadCharacteristic { value: payload })
        .await?;
    // After the tree so its InterfacesAdded covers everything at once.
    server.at(APP_PATH, ObjectManager).await?;
    server.at(ADV_PATH, Advertisement).await?;
    Ok(())
}

/// A running BLE pairing beacon.
pub struct BleBeacon {
    conn: Connection,
    adapter: OwnedObjectPath,
}

fn describe(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(name, Some(msg), _) => format!("{name}: {msg}"),
        other => other.to_string(),
    }
}

async fn find_adapter(conn: &Connection) -> Result<OwnedObjectPath, String> {
    let om = fdo::ObjectManagerProxy::builder(conn)
        .destination(BLUEZ)
        .path("/")
        .build()
        .await
        .map_err(describe)?;
    let objects = om.get_managed_objects().await.map_err(|e| {
        format!("BlueZ is not reachable on the system bus ({e}); is bluetooth.service running?")
    })?;
    let mut adapters: Vec<_> = objects
        .into_iter()
        .filter(|(_, ifaces)| {
            ifaces.contains_key("org.bluez.GattManager1")
                && ifaces.contains_key("org.bluez.LEAdvertisingManager1")
        })
        .collect();
    adapters.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    if adapters.is_empty() {
        return Err("no Bluetooth LE adapter found".into());
    }
    let powered = |ifaces: &HashMap<_, HashMap<String, OwnedValue>>| {
        ifaces
            .get("org.bluez.Adapter1")
            .and_then(|p| p.get("Powered"))
            .and_then(|v| bool::try_from(v).ok())
            .unwrap_or(false)
    };
    adapters
        .iter()
        .find(|(_, i)| powered(i))
        .map(|(p, _)| p.clone())
        .ok_or_else(|| {
            format!(
                "Bluetooth adapter {} is off (try `bluetoothctl power on`)",
                adapters[0].0.as_str().rsplit('/').next().unwrap_or("?")
            )
        })
}

impl BleBeacon {
    /// Starts advertising `payload` over BLE.
    pub fn start(payload: [u8; PAYLOAD_LEN]) -> Result<Self, String> {
        zbus::block_on(async {
            let conn = Connection::system()
                .await
                .map_err(|e| format!("cannot open the system D-Bus: {e}"))?;
            let adapter = find_adapter(&conn).await?;
            export(&conn, payload).await.map_err(describe)?;
            let no_opts: HashMap<&str, OwnedValue> = HashMap::new();
            let app = ObjectPath::from_static_str_unchecked(APP_PATH);
            let adv = ObjectPath::from_static_str_unchecked(ADV_PATH);
            conn.call_method(
                Some(BLUEZ),
                adapter.as_str(),
                Some("org.bluez.GattManager1"),
                "RegisterApplication",
                &(&app, &no_opts),
            )
            .await
            .map_err(|e| format!("GATT registration failed: {}", describe(e)))?;
            let beacon = Self { conn, adapter };
            beacon
                .conn
                .call_method(
                    Some(BLUEZ),
                    beacon.adapter.as_str(),
                    Some("org.bluez.LEAdvertisingManager1"),
                    "RegisterAdvertisement",
                    &(&adv, &no_opts),
                )
                .await
                .map_err(|e| format!("BLE advertising failed: {}", describe(e)))?;
            Ok(beacon)
        })
    }

    pub fn adapter_name(&self) -> &str {
        self.adapter.as_str().rsplit('/').next().unwrap_or("?")
    }
}

impl Drop for BleBeacon {
    fn drop(&mut self) {
        let conn = &self.conn;
        let adapter = self.adapter.as_str();
        zbus::block_on(async {
            let adv = ObjectPath::from_static_str_unchecked(ADV_PATH);
            let app = ObjectPath::from_static_str_unchecked(APP_PATH);
            let _ = conn
                .call_method(
                    Some(BLUEZ),
                    adapter,
                    Some("org.bluez.LEAdvertisingManager1"),
                    "UnregisterAdvertisement",
                    &(&adv,),
                )
                .await;
            let _ = conn
                .call_method(
                    Some(BLUEZ),
                    adapter,
                    Some("org.bluez.GattManager1"),
                    "UnregisterApplication",
                    &(&app,),
                )
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks the exported tree the way BlueZ reads it, on the session bus
    /// (skipped where there is none, e.g. CI containers).
    #[test]
    fn exported_tree_matches_bluez_expectations() {
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
            eprintln!("no session bus; skipping");
            return;
        }
        let payload: [u8; PAYLOAD_LEN] = std::array::from_fn(|i| i as u8);
        let server = zbus::block_on(async {
            let c = Connection::session().await.unwrap();
            export(&c, payload).await.unwrap();
            c
        });
        let name = server.unique_name().unwrap().to_string();
        // Served from the connection's own executor while this thread is
        // outside any block_on (the CLI then blocks on the control socket).
        let client = std::thread::spawn(move || {
            zbus::block_on(async move {
                let c = Connection::session().await.unwrap();
                let om = fdo::ObjectManagerProxy::builder(&c)
                    .destination(name.as_str())
                    .path(APP_PATH)
                    .build()
                    .await
                    .unwrap();
                let objs = om.get_managed_objects().await.unwrap();
                let ch = &objs
                    .iter()
                    .find(|(p, _)| p.as_str() == CHAR_PATH)
                    .expect("characteristic exported")
                    .1["org.bluez.GattCharacteristic1"];
                assert_eq!(
                    String::try_from(ch["UUID"].clone()).unwrap(),
                    PAYLOAD_CHAR_UUID
                );
                let svc = &objs
                    .iter()
                    .find(|(p, _)| p.as_str() == SERVICE_PATH)
                    .expect("service exported")
                    .1["org.bluez.GattService1"];
                assert!(bool::try_from(&svc["Primary"]).unwrap());

                let read = |offset: u16| {
                    let c = c.clone();
                    let name = name.clone();
                    async move {
                        let mut opts: HashMap<&str, zbus::Variant> = HashMap::new();
                        opts.insert("offset", offset.into());
                        c.call_method(
                            Some(name.as_str()),
                            CHAR_PATH,
                            Some("org.bluez.GattCharacteristic1"),
                            "ReadValue",
                            &(opts,),
                        )
                        .await
                        .unwrap()
                        .body()
                        .deserialize::<Vec<u8>>()
                        .unwrap()
                    }
                };
                assert_eq!(read(0).await, payload.to_vec());
                assert_eq!(read(22).await, payload[22..].to_vec());

                let adv = c
                    .call_method(
                        Some(name.as_str()),
                        ADV_PATH,
                        Some("org.freedesktop.DBus.Properties"),
                        "Get",
                        &("org.bluez.LEAdvertisement1", "ServiceUUIDs"),
                    )
                    .await
                    .unwrap();
                let v: OwnedValue = adv.body().deserialize().unwrap();
                assert_eq!(
                    Vec::<String>::try_from(v).unwrap(),
                    vec![SERVICE_UUID.to_string()]
                );
            })
        });
        client.join().unwrap();
        drop(server);
    }
}
