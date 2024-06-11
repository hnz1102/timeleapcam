use anyhow;
use std::{thread, time::Duration};
use esp_camera_rs::Camera;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::gpio::{PinDriver, Gpio2};
use esp_idf_svc::wifi::EspWifi;
use esp_idf_sys::camera;
use log::info;
use std::net::Ipv4Addr;

use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault, EspNvs, NvsPartitionId};
use esp_idf_svc::sntp::{EspSntp, SyncStatus, SntpConf, OperatingMode, SyncMode};
use esp_idf_hal::adc::{config::Config as AdcConfig, AdcChannelDriver, AdcDriver};
use std::time::{SystemTime, UNIX_EPOCH};
use chrono::{DateTime, Utc};
use chrono::{Local, Duration as ChronoDuration, FixedOffset, NaiveDate, Datelike, Timelike};

//use esp_hal_procmacros::ram;

mod wifi;
mod capture;
mod autofocus;
mod emmc;
mod imagefiles;
mod server;
mod config;
mod touchpad;
mod monitoring;

use touchpad::{TouchPad, KeyEvent, Key};
use config::ConfigData;
use capture::Capture;
use emmc::EMMCHost;
use server::LeapTime;
use monitoring::Monitoring;

#[derive(PartialEq)]
enum SleepMode {
    SleepModeLight,
    SleepModeDeep,
}

const MAX_NVS_STR_SIZE : usize = 3072;

#[link_section = ".rtc.data"]
static mut IMAGE_COUNT_ID: u32 = 0;

#[link_section = ".rtc.data"]
static mut DEEP_SLEEP_AUTO_CAPTURE: bool = false;

#[link_section = ".rtc.data"]
static mut NEXT_CAPTURE_TIME: u64 = 0;

#[link_section = ".rtc.data"]
static mut DURATION_TIME: u32 = 0;

#[link_section = ".rtc.data"]
static mut CURRENT_RESOLUTION: u32 = 0;

#[link_section = ".rtc.data"]
static mut CURRENT_TRACK_ID: u32 = 0;

#[link_section = ".rtc.data"]
static mut LAST_CAPTURE_TIME: u64 = 0;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    // Peripherals Initialize
    let peripherals = Peripherals::take().unwrap();

    // eMMC and Camera Power On (low active)
    let mut emmc_cam_power = PinDriver::output(peripherals.pins.gpio3).unwrap();
    emmc_cam_power.set_high().expect("Set emmc_cam_power high failure");
    // emmc_cam_power.set_low().expect("Set emmc_cam_power low failure");

    // TouchPad
    let mut touchpad = TouchPad::new();
    touchpad.start();
    thread::sleep(Duration::from_millis(100));

    // operating mode
    let mut operating_mode = false;

    // wakeup reason
    let wakeup_reason : u32;
    unsafe {
        wakeup_reason = esp_idf_sys::esp_sleep_get_wakeup_cause();
        info!("Wakeup reason: {:?}", wakeup_reason);
        match wakeup_reason {
            esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_TOUCHPAD => {
                operating_mode = true;
                info!("Wakeup by touchpad");
            },
            esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_TIMER => {
                info!("Wakeup by timer");
            },
            esp_idf_sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_UNDEFINED => {
                info!("Power ON boot");
                if touchpad.get_touchpad_status(Key::Center) {
                    info!("Erase NVS flash...");
                    esp_idf_sys::nvs_flash_erase();
                    thread::sleep(Duration::from_millis(1000));
                }    
                esp_idf_sys::nvs_flash_init();
                operating_mode = true;
            },
            _ => {
                info!("Wakeup by Unknown");
            }
        }
    }
    // Initialize Configuration Data
    let mut config_data = ConfigData::new();

    // XIAO ESP32S3 is always in operating mode
    operating_mode = true;

    // Initialize NVS
    let nvs_default_partition = EspNvsPartition::<NvsDefault>::take().unwrap();
    let mut nvs = match EspNvs::new(nvs_default_partition, "storage", true) {
        Ok(nvs) => { info!("NVS storage area initialized"); nvs },
        Err(ref e) => {
            panic!("NVS initialization failed {:?}", e); }
    };

    // Load config
    let mut nvs_buf : [u8 ; MAX_NVS_STR_SIZE] = [0; MAX_NVS_STR_SIZE];
    let nvs_value = match nvs.get_str("config", &mut nvs_buf){
        Ok(value) => { info!("Try to read NVS config"); value },
        Err(ref e) => { info!("NVS config not found {:?}", e); None }
    };
    if nvs_value == None {
        info!("NVS config not found. Set default config");
        set_default_config(&mut config_data, &mut nvs);
        thread::sleep(Duration::from_millis(1000));
    }
    else {
        // info!("NVS config found {:?}", nvs_value);
        match config_data.load_config(nvs_value) {
            Ok(_) => { info!("Config load success"); },
            Err(ref e) => { 
                info!("Config load failed {:?}", e);
                set_default_config(&mut config_data, &mut nvs);
                thread::sleep(Duration::from_millis(1000));
            },
        }    
    }

    // Initialize ADC
    // let mut adc = AdcDriver::new(peripherals.adc1, &AdcConfig::new().calibration(true))?;
    // let mut adc_pin : AdcChannelDriver<'_, {esp_idf_sys::adc_atten_t_ADC_ATTEN_DB_11}, Gpio2> = AdcChannelDriver::new(peripherals.pins.gpio2)?;
    // let battery_voltage : f32 =  adc.read(&mut adc_pin).unwrap() as f32 * 2.0 / 1000.0;
    // info!("Battery Voltage: {:.2}V", battery_voltage);
    // emmc initialize
    let mut emmc = EMMCHost::new();
    emmc.mount(); 
    // emmc.format();
    // imagefiles::delete_all_files(Path::new("/eMMC"));

    let mut server_info = server::ControlServerInfo::new();
    // config_data
    // info!("Start config_data: {:?}", config_data);
    info!("Auto Capture: {:?}", unsafe { DEEP_SLEEP_AUTO_CAPTURE } );
    info!("Image Count ID: {:?}", unsafe { IMAGE_COUNT_ID });

    let mut current_resolution = camera::framesize_t_FRAMESIZE_VGA;
    if operating_mode {
        // operating mode
        server_info.auto_capture = config_data.auto_capture;
        server_info.capture_started = config_data.auto_capture;
        server_info.duration = config_data.duration;
        server_info.resolution = config_data.resolution;
        server_info.track_id = config_data.track_id;
        server_info.timezone = config_data.timezone_offset;
        server_info.idle_in_sleep_time = config_data.idle_in_sleep_time;
    }
    else {
        // wakeup
        server_info.capture_started = config_data.auto_capture;
        server_info.duration = unsafe { DURATION_TIME };
        server_info.resolution = unsafe { CURRENT_RESOLUTION };
        server_info.track_id = unsafe { CURRENT_TRACK_ID };
        server_info.timezone = config_data.timezone_offset;
        server_info.idle_in_sleep_time = config_data.idle_in_sleep_time;
        current_resolution = server_info.resolution;

    }
    server_info.last_capture_date_time = SystemTime::UNIX_EPOCH + Duration::from_secs(unsafe { LAST_CAPTURE_TIME });
    server_info.query_prompt = config_data.query_prompt.clone();
    server_info.query_openai = config_data.query_openai;
    server_info.openai_model = config_data.model.clone();
    // let mut current_resolution = server_info.resolution;
    let mut next_capture_time = UNIX_EPOCH + Duration::from_secs(unsafe { NEXT_CAPTURE_TIME });
    let mut capture_count = unsafe { IMAGE_COUNT_ID };
    let mut current_track_id = server_info.track_id;
    let mut current_duration = server_info.duration;
    let operating_mode_start_time = SystemTime::now();
    let dt_utc : DateTime<Utc> = DateTime::<Utc>::from(next_capture_time);
    let fixed_offset = FixedOffset::east_opt(config_data.timezone_offset * 3600).unwrap();
    let dt_local = DateTime::<Local>::from_naive_utc_and_offset(dt_utc.naive_utc(), fixed_offset);
    info!("Next Capture Time: {}", dt_local.format("%Y-%m-%d %H:%M:%S"));

    // current_settings into server_info
    server_info.leap_time = LeapTime {
        year: dt_local.year() as u32,
        month: dt_local.month(),
        day: dt_local.day(),
        hour: dt_local.hour(),
        minute: dt_local.minute(),
        second: dt_local.second(),
    };

    // wifi initialize
    let mut wifi_dev : Result<Box<EspWifi>, anyhow::Error> = Result::Err(anyhow::anyhow!("WiFi not connected"));
    let mut server : Option<server::ControlServer> = match operating_mode || config_data.query_openai {
        true => {
            wifi_dev = wifi::wifi_connect(peripherals.modem, &config_data.wifi_ssid, &config_data.wifi_psk);
            match &wifi_dev {
                Ok(_) => { info!("WiFi connected"); },
                Err(ref e) => { info!("{:?}", e); }
            }
            let rssi = wifi::get_rssi();
            info!("RSSI: {}dBm", rssi);
        
            // Get my IP address
            let mut ip_addr : Ipv4Addr; 
            loop {
                ip_addr = wifi_dev.as_ref().unwrap().sta_netif().get_ip_info().unwrap().ip;
                if ip_addr != Ipv4Addr::new(0, 0, 0, 0) {
                    break;
                }
                info!("Waiting for WiFi connection...");
                thread::sleep(Duration::from_secs(1));
            }
            
            // NTP Server
            let sntp_conf = SntpConf {
                servers: ["time.aws.com",
                            "time.google.com",
                            "time.cloudflare.com",
                            "ntp.nict.jp"],
                operating_mode: OperatingMode::Poll,
                sync_mode: SyncMode::Immediate,
            };
            let ntp = EspSntp::new(&sntp_conf).unwrap();

            // NTP Sync
            // let now = SystemTime::now();
            // if now.duration_since(UNIX_EPOCH).unwrap().as_millis() < 1700000000 {
            info!("NTP Sync Start..");
            // wait for sync
            while ntp.get_sync_status() != SyncStatus::Completed {
                thread::sleep(Duration::from_millis(10));
            }
            let now = SystemTime::now();
            let dt_now : DateTime<Utc> = now.into();
            let formatted = format!("{}", dt_now.format("%Y-%m-%d %H:%M:%S"));
            info!("NTP Sync Completed: {}", formatted);

            // HTTP Server
            let mut server = match server::ControlServer::new(&server_info) {
                Ok(server_ctx) => {
                    info!("HTTP Server started");
                    server_ctx
                }
                Err(e) => {
                    info!("Failed to start HTTP Server: {:?}", e);
                    return Result::Err(anyhow::anyhow!("Failed to start HTTP Server"));
                }
            };
            server.start();
            server.set_server_info(server_info.clone());
            Some(server)
        },
        false => {
            None
        }
    };

    // Initialize the camera
    let cam = Camera::new(
        peripherals.pins.gpio43,    // PWDN
        peripherals.pins.gpio0,     // RESET
        peripherals.pins.gpio10,    // XCLK
        peripherals.pins.gpio40,    // SIOD
        peripherals.pins.gpio39,    // SIOC
        peripherals.pins.gpio15,    // Y2
        peripherals.pins.gpio17,    // Y3
        peripherals.pins.gpio18,    // Y4
        peripherals.pins.gpio16,    // Y5
        peripherals.pins.gpio14,    // Y6
        peripherals.pins.gpio12,    // Y7
        peripherals.pins.gpio11,    // Y8
        peripherals.pins.gpio48,    // Y9
        peripherals.pins.gpio38,    // VSYNC
        peripherals.pins.gpio47,    // HREF
        peripherals.pins.gpio13,    // PCLK
        20000000,                    // XCLK frequency
        10,                         // JPEG quality
        2,                         // Frame buffer count
        camera::camera_grab_mode_t_CAMERA_GRAB_LATEST,        // grab mode
        // camera::camera_grab_mode_t_CAMERA_GRAB_WHEN_EMPTY,    // grab mode
        current_resolution,        // frame size have to be maximum resolution
    );
    let camera_device : Camera = match cam {
        Ok(cam) => {
            cam
        }
        Err(e) => {
            info!("Failed to initialize camera: {:?}", e);
            return Result::Err(anyhow::anyhow!("Failed to initialize camera"));
        }
    };

    let mut capture = Capture::new(camera_device, "/eMMC");
    capture.start();
    let monitoring_thread = Monitoring::new(config_data.model.clone(), config_data.api_key.clone());
    monitoring_thread.set_post_access_token(config_data.post_account.clone(),
        config_data.post_access_token.clone(), config_data.post_message_trigger.clone());
    monitoring_thread.set_storage_access_token(config_data.storage_account.clone(),
        config_data.storage_access_token.clone());
    monitoring_thread.start();
    if operating_mode {
        unsafe { DEEP_SLEEP_AUTO_CAPTURE = false; }
        config_data.auto_capture = false;
    }

    loop {
        // imagefiles::list_files(Path::new("/eMMC"));
        if config_data.auto_capture || unsafe { DEEP_SLEEP_AUTO_CAPTURE } {
            server_info.capture_started = true;
        }

        if operating_mode {
            let rssi = wifi::get_rssi();
            if rssi == 0 {
                wifi_reconnect(&mut wifi_dev.as_mut().unwrap());
            }
            server.as_mut().unwrap().set_current_rssi(rssi);
            // read battery voltage
            // let battery_voltage : f32 =  adc.read(&mut adc_pin).unwrap() as f32 * 2.0 / 1000.0;
            // server.as_mut().unwrap().set_current_battery_voltage(battery_voltage);
            server_info = server.as_mut().unwrap().get_server_info().clone();

            // check save config
            if server_info.need_to_save {
                server_info.need_to_save = false;
                config_data.auto_capture = server_info.auto_capture;
                config_data.duration = server_info.duration;
                config_data.resolution = server_info.resolution;
                config_data.track_id = server_info.track_id;
                config_data.timezone_offset = server_info.timezone;
                config_data.idle_in_sleep_time = server_info.idle_in_sleep_time;
                config_data.query_openai = server_info.query_openai;
                config_data.query_prompt = server_info.query_prompt.clone();
                config_data.model = server_info.openai_model.clone();
                let save_config = config_data.get_all_config();
                let toml_cfg = convert_config_to_toml_string(&save_config);
                match nvs.set_str("config", toml_cfg.as_str()) {
                    Ok(_) => { info!("Save config"); },
                    Err(ref e) => { info!("Set default config failed {:?}", e); }
                }
                server.as_mut().unwrap().set_server_info(server_info.clone());
                thread::sleep(Duration::from_millis(1000));
                unsafe {
                    esp_idf_sys::esp_restart();
                }
            }
            current_duration = server_info.duration;
            // server.as_mut().unwrap().set_server_capture_started(server_info.capture_started);
            // if server_info.latest_access_time.elapsed().unwrap().as_secs() > config_data.idle_in_sleep_time as u64 {
            //     if operating_mode_start_time.elapsed().unwrap().as_secs() > config_data.idle_in_sleep_time as u64 {
            //         operating_mode = false;
            //         // emmc_cam_power.set_high().expect("Set emmc_cam_power high failure");
            //         deep_and_light_sleep_start(SleepMode::SleepModeDeep, 0);
            //     }
            // }    
        }

        // let key_event = touchpad.get_key_event_and_clear();
        // for key in key_event {
        //     match key {
        //         KeyEvent::CenterKeyUp => {
        //             operating_mode = false;
        //         },
        //         _ => {
        //         }
        //     }
        // }
    
        if server_info.resolution != current_resolution {
            info!("Resolution changed: {} -> {}", current_resolution, server_info.resolution);
            current_resolution = server_info.resolution;
            capture.change_resolution(current_resolution);
        }
        if server_info.capture_started {
            info!("Capture started");
            if current_track_id != server_info.track_id {
                info!("Track ID changed: {} -> {}", current_track_id, server_info.track_id); 
                current_track_id = server_info.track_id;
                capture_count = 0;
            }
            if capture_count == 0 {
                // auto focus trigger
                capture.autofocus_request();
                next_capture_time = SystemTime::now();
            }
            info!("Capture Track ID: {} Count: {} Resolution: {}", current_track_id, capture_count, current_resolution);
            capture.capture_request(current_track_id, capture_count);
            loop {
                if capture.get_capture_status() {
                    info!("Capture done");
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            if server_info.query_openai {
                monitoring_thread.set_query_start(config_data.query_prompt.clone(), current_track_id, capture_count);
                loop {
                    if !monitoring_thread.get_query_status() {
                        let reply = monitoring_thread.get_query_reply();
                        info!("Query reply: {}", reply);
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            }
            let capture_info = capture.get_capture_info();
            if capture_info.status {
                info!("Write done {}: width:{} height:{} image_size:{}", capture_count, capture_info.width, capture_info.height, capture_info.size);
                server_info.last_capture_date_time = SystemTime::now();
                capture_count += 1;
                server_info.current_capture_id = capture_count;
            }

            next_capture_time = get_next_wake_time(server_info.leap_time, server_info.timezone, next_capture_time, server_info.duration);
            // parse next_capture_time to string
            let dt_utc : DateTime<Utc> = DateTime::<Utc>::from(next_capture_time);
            let fixed_offset = FixedOffset::east_opt(server_info.timezone * 3600).unwrap();
            let dt_local = DateTime::<Local>::from_naive_utc_and_offset(dt_utc.naive_utc(), fixed_offset);
            info!("Next Capture Time: {}", dt_local.format("%Y-%m-%d %H:%M:%S"));
            let sleep_time = match next_capture_time.duration_since(SystemTime::now()) {
                Ok(duration) => duration,
                Err(e) => {
                    info!("Time calculation error: {:?}", e);
                    Duration::from_secs(1)
                }
            };
            info!("Sleep: {:?}", sleep_time);
            match sleep_time.as_secs() {
                0 => {
                    thread::sleep(Duration::from_millis(1000));
                },
                1..=60 => { // 0-60s is OS sleep
                    thread::sleep(sleep_time);
                }
                _ => {
                    unsafe {
                        IMAGE_COUNT_ID = capture_count;
                        DEEP_SLEEP_AUTO_CAPTURE = true;
                        NEXT_CAPTURE_TIME = next_capture_time.duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;
                        DURATION_TIME = current_duration;
                        CURRENT_RESOLUTION = current_resolution;
                        CURRENT_TRACK_ID = current_track_id;
                        LAST_CAPTURE_TIME = server_info.last_capture_date_time.duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;
                    }
                    // emmc_cam_power.set_high().expect("Set emmc_cam_power high failure");
                    deep_and_light_sleep_start(SleepMode::SleepModeDeep, sleep_time.as_secs());
                }
            }
        }
        else {
            if capture_count > 0 {
                capture_count = 0;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

// duration: > 0: Capture every duration seconds, = 0: Capture at specific time by LeapTime
fn get_next_wake_time(lt: LeapTime, timezone: i32, mut next_capture_time: SystemTime, duration: u32) -> SystemTime {
    if duration > 0 {
        next_capture_time = next_capture_time + Duration::from_secs(duration as u64);
        return next_capture_time;
    }
    else {
        let offset = FixedOffset::east_opt(timezone * 60 * 60).unwrap();
        let now_with_offset = Local::now().with_timezone(&offset);
        // parse now_with_offset to string
        let now_str = now_with_offset.format("%Y-%m-%d %H:%M:%S").to_string();
        info!("Now with offset: {}", now_str);
        let now_naive = now_with_offset.naive_local();
        if lt.year != 2024 && lt.month != 1 && lt.day != 1 {
            // LeapTime is in the future, get duration from now to LeapTime. leap_time is local time.
            let leap_time = NaiveDate::from_ymd_opt(lt.year as i32, lt.month, lt.day).unwrap()
            .and_hms_opt(lt.hour, lt.minute, lt.second).unwrap().and_local_timezone(offset).unwrap();
            if now_naive < leap_time.naive_local() {
                // convert leap_time to UTC as DateTime
                next_capture_time = leap_time.into();
            }
        }
        else {
            // LeapTime is interval time, get duration from now to LeapTime
            let today = now_naive.date();
            let tomorrow = today + ChronoDuration::days(1);
            let today_leap_time = NaiveDate::from_ymd_opt(today.year(), today.month(), today.day())
                .unwrap().and_hms_opt(lt.hour, lt.minute, lt.second).unwrap().and_local_timezone(offset).unwrap();
            if now_naive < today_leap_time.naive_local() {
                // today
                next_capture_time = today_leap_time.into();
            }
            else {
                // tomorrow
                let tomorrow_leap_time = NaiveDate::from_ymd_opt(tomorrow.year(), tomorrow.month(), tomorrow.day())
                    .unwrap().and_hms_opt(lt.hour, lt.minute, lt.second).unwrap().and_local_timezone(offset).unwrap();
                next_capture_time = tomorrow_leap_time.into();
            }
        }
    }
    next_capture_time
}

fn deep_and_light_sleep_start(sleep_mode: SleepMode, wakeup_interval: u64) {
    info!("Sleep Now...");
    unsafe {
        esp_idf_sys::esp_wifi_stop();
    }
    thread::sleep(Duration::from_millis(1000));
    unsafe {
        // light sleep mode
        if sleep_mode == SleepMode::SleepModeLight {
            // gpio wakeup enable
            // esp_idf_sys::gpio_wakeup_enable(GPIO_WAKEUP_INT_PIN_4, esp_idf_sys::gpio_int_type_t_GPIO_INTR_LOW_LEVEL);
            // esp_idf_sys::esp_sleep_enable_gpio_wakeup();
            // wakeup from rtc timer
            if wakeup_interval > 0 {
                esp_idf_sys::esp_sleep_enable_timer_wakeup(wakeup_interval * 1000 * 1000);
            }
        }
        else {
            info!("Deep Sleep Start");
            esp_idf_sys::esp_sleep_enable_gpio_wakeup();            
            esp_idf_sys::esp_sleep_enable_touchpad_wakeup();
            if wakeup_interval > 0 {
                esp_idf_sys::esp_sleep_enable_timer_wakeup(wakeup_interval * 1000 * 1000);
            }
            esp_idf_sys::esp_deep_sleep_start();
        }

        // deep sleep mode (not here)
        if sleep_mode == SleepMode::SleepModeLight {
            let _result = esp_idf_sys::esp_light_sleep_start();
            // gpio interrupt enable
            // esp_idf_sys::gpio_set_intr_type(GPIO_WAKEUP_INT_PIN_4, esp_idf_sys::gpio_int_type_t_GPIO_INTR_ANYEDGE);
        }
    }
}

fn wifi_reconnect(wifi_dev: &mut EspWifi) -> bool{
    // display on
    unsafe {
        esp_idf_sys::esp_wifi_start();
    }
    match wifi_dev.connect() {
        Ok(_) => { info!("Wifi connected"); true},
        Err(ref e) => { info!("{:?}", e); false }
    }
}

fn set_default_config<T : NvsPartitionId>(config: &mut ConfigData, nvs: &mut EspNvs<T>){
    let default_config = config.set_default_config();
    let toml_cfg = convert_config_to_toml_string(&default_config);
    match nvs.set_str("config", toml_cfg.as_str()) {
        Ok(_) => { info!("Set default config"); },
        Err(ref e) => { info!("Set default config failed {:?}", e); }
    }
    match config.load_config(Some(toml_cfg.as_str())) {
        Ok(_) => { info!("Config load success"); },
        Err(ref e) => { info!("Config load failed {:?}", e);
            unsafe {
                esp_idf_sys::nvs_flash_erase();
            }
        }
    };
}

fn convert_config_to_toml_string(keyval: &Vec<(String, String)>) -> String {
    let mut toml_string = String::new();
    for it in keyval {
        toml_string.push_str(&format!("{} = \"{}\"\n", it.0, it.1));
    }
    toml_string
}

// save config
#[allow(dead_code)]
fn save_config<T : NvsPartitionId>(config: &ConfigData, nvs: &mut EspNvs<T>) {
    let save_config = config.get_all_config();
    let toml_cfg = convert_config_to_toml_string(&save_config);
    match nvs.set_str("config", toml_cfg.as_str()) {
        Ok(_) => { info!("Save config"); },
        Err(ref e) => { info!("Set default config failed {:?}", e); }
    }
}