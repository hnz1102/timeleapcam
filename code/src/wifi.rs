use std::time::Duration;
use std::thread;

use esp_idf_hal::peripheral;
use esp_idf_svc::{eventloop::EspSystemEventLoop, wifi::EspWifi};
use esp_idf_sys;

use embedded_svc::wifi::{ClientConfiguration, Configuration};
use anyhow::bail;
use anyhow::Result;
use log::*;
use std::str::FromStr;

pub fn wifi_connect<'d> (
    modem: impl peripheral::Peripheral<P = esp_idf_hal::modem::Modem> + 'static,
    ssid: &'d str,
    pass: &'d str,
) -> Result<Box<EspWifi<'d>>> {

    if ssid.is_empty() || pass.is_empty() {
        bail!("SSID or password is empty");
    }
    let sys_event_loop = EspSystemEventLoop::take().unwrap();
    let mut wifi = Box::new(EspWifi::new(modem, sys_event_loop.clone(), None).unwrap());

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: heapless::String::<32>::from_str(ssid).unwrap(),
        password: heapless::String::<64>::from_str(pass).unwrap(),
        ..Default::default()
    })).unwrap();

    wifi.start().unwrap();
    wifi.connect()?;
    let mut timeout = 0;
    loop {
        if wifi.is_connected().unwrap(){
            info!("Wifi connected");
            break;
        }
        thread::sleep(Duration::from_secs(1));
        timeout += 1;
        if timeout > 30 {
            info!("Wifi could not be connected.");
            // wifi could not be connected, but we can use the wifi object to reconnect
            break;
        }
    }
    Ok(wifi)
}

pub fn get_rssi() -> i32 {
    unsafe {
        let mut rssi : i32 = 0;
        esp_idf_sys::esp_wifi_sta_get_rssi(&mut rssi);
        rssi
    }
}