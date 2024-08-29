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
  
    let sys_event_loop = EspSystemEventLoop::take().unwrap();
    let mut wifi = Box::new(EspWifi::new(modem, sys_event_loop.clone(), None).unwrap());

    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: heapless::String::<32>::from_str(ssid).unwrap(),
        password: heapless::String::<64>::from_str(pass).unwrap(),
        ..Default::default()
    })).unwrap();

    wifi.start().unwrap();

    // XIAO ESP32S3 has some issue with the default max tx power(80), so we have to set it to 66.
    // This is a workaround for the issue. If the default tx power is 80, then the wifi connection will fail.
    unsafe {
        let mut default_tx_power : i8 = 0;
        let _ = esp_idf_sys::esp_wifi_get_max_tx_power(&mut default_tx_power);
        info!("Max Tx Default Power {}", default_tx_power);
        let res = esp_idf_sys::esp_wifi_set_max_tx_power(60);
        info!("Set max tx power to 60, result: 0x{:x}", res);
    }

    wifi.connect()?;
    let mut timeout = 0;
    while !wifi.is_connected().unwrap(){
        thread::sleep(Duration::from_secs(1));
        timeout += 1;
        if timeout > 30 {
            bail!("Wifi could not be connected.");
        }
    }
    info!("Wifi connected");
    Ok(wifi)
}

pub fn get_rssi() -> i32 {
    unsafe {
        let mut rssi : i32 = 0;
        esp_idf_sys::esp_wifi_sta_get_rssi(&mut rssi);
        rssi
    }
}