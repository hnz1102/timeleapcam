#![allow(dead_code, unused_imports)]

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

#[link_section = ".rtc.data"]
static mut LAST_POSTED_TIME: u64 = 0;

#[link_section = ".rtc.data"]
static mut CAPTURE_START_TIME: u64 = 0;

#[link_section = ".rtc.data"]
static mut CAPTURE_END_TIME: u64 = 0;

#[link_section = ".rtc.data"]
static mut LAST_STATUS_POSTED_TIME: u64 = 0;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    // Peripherals Initialize
    let peripherals = Peripherals::take().unwrap();

    // eMMC and Camera Power On (low active)
    let mut emmc_cam_power = PinDriver::output(peripherals.pins.gpio44).unwrap();
    emmc_cam_power.set_low().expect("Set emmc_cam_power low failure");

    // TouchPad
    let mut touchpad = TouchPad::new();
    touchpad.start();
    thread::sleep(Duration::from_millis(1000));

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

    // Initialize Temperature Sensor
    let mut config = esp_idf_svc::hal::sys::temperature_sensor_config_t::default();
    let mut temp_sensor_ptr : *mut esp_idf_svc::sys::temperature_sensor_obj_t =
             std::ptr::null_mut() as *mut esp_idf_svc::sys::temperature_sensor_obj_t;
    unsafe {
        esp_idf_svc::hal::sys::temperature_sensor_install(&mut config, &mut temp_sensor_ptr);
        esp_idf_svc::hal::sys::temperature_sensor_enable(&mut *temp_sensor_ptr);
    }

    // Initialize ADC
    let mut adc = AdcDriver::new(peripherals.adc1, &AdcConfig::new().calibration(true))?;
    let mut adc_pin : AdcChannelDriver<'_, {esp_idf_sys::adc_atten_t_ADC_ATTEN_DB_11}, Gpio2> = AdcChannelDriver::new(peripherals.pins.gpio2)?;
    let battery_voltage : f32 =  adc.read(&mut adc_pin).unwrap() as f32 * 2.0 / 1000.0;
    info!("Battery Voltage: {:.2}V", battery_voltage);
    // emmc initialize
    let mut emmc = EMMCHost::new();
    let mut mount_retry = 0;
    loop {
        match emmc.mount() {
            Ok(_) => { info!("eMMC/SDCard mounted");
                break;
            },
            Err(e) => {
                // format eMMC
                emmc.format();
                info!("eMMC/SDCard mount failed {:?}", e);
                mount_retry += 1;
                if mount_retry > 3 {
                    info!("eMMC/SDCard mount failed.");
                    break;
                }
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    // imagefiles::delete_all_files(Path::new("/eMMC"));

    let mut server_info = server::ControlServerInfo::new();
    // config_data
    // info!("Start config_data: {:?}", config_data);
    info!("Auto Capture: {:?}", unsafe { DEEP_SLEEP_AUTO_CAPTURE } );
    info!("Image Count ID: {:?}", unsafe { IMAGE_COUNT_ID });

    let mut current_resolution = camera::framesize_t_FRAMESIZE_QSXGA;
    if operating_mode {
        // operating mode
        server_info.duration = config_data.duration;
        server_info.resolution = config_data.resolution;
        server_info.track_id = config_data.track_id;
    }
    else {
        // wakeup
        server_info.duration = unsafe { DURATION_TIME };
        server_info.resolution = unsafe { CURRENT_RESOLUTION };
        server_info.track_id = unsafe { CURRENT_TRACK_ID };
        current_resolution = server_info.resolution;

    }
    server_info.auto_capture = config_data.auto_capture;
    server_info.idle_in_sleep_time = config_data.idle_in_sleep_time;
    server_info.timezone = config_data.timezone_offset;
    server_info.autofocus_once = config_data.autofocus_once;
    server_info.last_capture_date_time = SystemTime::UNIX_EPOCH + Duration::from_secs(unsafe { LAST_CAPTURE_TIME });
    server_info.last_posted_date_time = SystemTime::UNIX_EPOCH + Duration::from_secs(unsafe { LAST_POSTED_TIME });
    server_info.capture_start_time = SystemTime::UNIX_EPOCH + Duration::from_secs(unsafe { CAPTURE_START_TIME });
    server_info.capture_end_time = SystemTime::UNIX_EPOCH + Duration::from_secs(unsafe { CAPTURE_END_TIME });    
    server_info.query_prompt = config_data.query_prompt.clone();
    server_info.query_openai = config_data.query_openai;
    server_info.openai_model = config_data.model.clone();
    server_info.status_report = config_data.status_report;
    server_info.status_report_interval = config_data.status_report_interval;
    server_info.post_interval = config_data.post_interval;
    server_info.capture_frames_at_once = config_data.capture_frames_at_once;
    server_info.overwrite_saved = config_data.overwrite_saved;
    let mut last_status_posted_time = SystemTime::UNIX_EPOCH + Duration::from_secs(unsafe { LAST_STATUS_POSTED_TIME });
    let mut next_capture_time = UNIX_EPOCH + Duration::from_secs(unsafe { NEXT_CAPTURE_TIME });
    let mut capture_id = unsafe { IMAGE_COUNT_ID };
    let mut current_track_id = server_info.track_id;
    let mut current_duration = server_info.duration;
    let dt_utc : DateTime<Utc> = DateTime::<Utc>::from(next_capture_time);
    let fixed_offset = FixedOffset::east_opt(config_data.timezone_offset * 3600).unwrap();
    let dt_local = DateTime::<Local>::from_naive_utc_and_offset(dt_utc.naive_utc(), fixed_offset);
    info!("Next Capture Time: {} Capture Count: {}", dt_local.format("%Y-%m-%d %H:%M:%S"), capture_id);

    let status_post_need = server_info.status_report && (capture_id % server_info.status_report_interval) == 0;
    // current_settings into server_info
    server_info.leap_time = LeapTime {
        year: -1,
        month: -1,
        day: config_data.leap_day,
        hour: config_data.leap_hour,
        minute: config_data.leap_minute,
        second: 0,
    };

    // wifi initialize
    let mut wifi_dev : Result<Box<EspWifi>, anyhow::Error> = Result::Err(anyhow::anyhow!("WiFi not connected"));
    let mut server_enalbed = false;
    let mut server : Option<server::ControlServer> = match operating_mode || config_data.query_openai || status_post_need {
        true => {
            wifi_dev = wifi::wifi_connect(peripherals.modem, &config_data.wifi_ssid, &config_data.wifi_psk);
            match &wifi_dev {
                Ok(_) => { info!("WiFi connected"); },
                Err(ref e) => { info!("{:?}", e); }
            }
            let rssi = wifi::get_rssi();
            info!("RSSI: {}dBm", rssi);
            // ssid
            let ssid = config_data.wifi_ssid.clone();
            info!("Connected SSID: {:?}", ssid);
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
            info!("My IP address: {}", ip_addr);
            
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
            server_enalbed = true;
            Some(server)
        },
        false => {
            None
        }
    };

    // Initialize the camera
    let xclk = match server_info.capture_frames_at_once == 0 && ! operating_mode {
        true => 5000000,
        false => 25000000,
    };
    let cam = Camera::new(
        peripherals.pins.gpio48,    // XCLK
        peripherals.pins.gpio42,    // SIOD
        peripherals.pins.gpio41,    // SIOC
        peripherals.pins.gpio11,    // Y2
        peripherals.pins.gpio18,    // Y3
        peripherals.pins.gpio17,    // Y4
        peripherals.pins.gpio10,    // Y5
        peripherals.pins.gpio12,    // Y6
        peripherals.pins.gpio14,    // Y7
        peripherals.pins.gpio47,    // Y8
        peripherals.pins.gpio38,    // Y9
        peripherals.pins.gpio40,    // VSYNC
        peripherals.pins.gpio39,    // HREF
        peripherals.pins.gpio13,    // PCLK
        xclk,                       // XCLK frequency
        10,                         // JPEG quality
        2,                          // Frame buffer count (back to 2 for double buffering)
        camera::camera_grab_mode_t_CAMERA_GRAB_LATEST,        // grab mode
        //camera::camera_grab_mode_t_CAMERA_GRAB_WHEN_EMPTY,    // grab mode
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
                                               config_data.storage_access_token.clone(),
                                               config_data.storage_signed_key.clone());
    monitoring_thread.set_last_posted_date_time(server_info.last_posted_date_time, server_info.post_interval);
    monitoring_thread.start();
    if operating_mode {
        unsafe { DEEP_SLEEP_AUTO_CAPTURE = false; }
        config_data.auto_capture = false;
        server_info.last_access_time = SystemTime::now();
        server.as_mut().unwrap().set_server_info(server_info.clone());
    }

    // led_ind.set_high().expect("Set indicator high failure");
    let mut one_shot = false;
    let mut movie_mode = false;
    let mut capture_indicator_on = false;
    loop {
        // imagefiles::list_files(Path::new("/eMMC"));
        if config_data.auto_capture || unsafe { DEEP_SLEEP_AUTO_CAPTURE } {
            server_info.capture_started = true;
        }
        // read battery voltage
        let battery_voltage : f32 =  adc.read(&mut adc_pin).unwrap() as f32 * 2.0 / 1000.0;

        if operating_mode {
            let rssi = wifi::get_rssi();
            if rssi == 0 {
                wifi_reconnect(&mut wifi_dev.as_mut().unwrap());
            }
            server.as_mut().unwrap().set_current_rssi(rssi);
            server.as_mut().unwrap().set_current_battery_voltage(battery_voltage);
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
                config_data.autofocus_once = server_info.autofocus_once;
                config_data.status_report = server_info.status_report;
                config_data.status_report_interval = server_info.status_report_interval;
                config_data.post_interval = server_info.post_interval;
                config_data.leap_day = server_info.leap_time.day;
                config_data.leap_hour = server_info.leap_time.hour;
                config_data.leap_minute = server_info.leap_time.minute;
                config_data.capture_frames_at_once = server_info.capture_frames_at_once;
                config_data.overwrite_saved = server_info.overwrite_saved;
                let save_config = config_data.get_all_config();
                let toml_cfg = convert_config_to_toml_string(&save_config);
                match nvs.set_str("config", toml_cfg.as_str()) {
                    Ok(_) => { info!("Save config"); },
                    Err(ref e) => { info!("Set default config failed {:?}", e); }
                }
                server.as_mut().unwrap().set_server_info(server_info.clone());
            }
            one_shot = server.as_mut().unwrap().get_one_shot();
            current_duration = server_info.duration;
            server.as_mut().unwrap().set_server_capture_started(server_info.capture_started);
            if !server_info.capture_started {
                // when idle, check last access time
                if server_info.last_access_time.duration_since(UNIX_EPOCH).unwrap().as_millis() > 1700000000 {
                    let last_access_time = server_info.last_access_time.elapsed().unwrap().as_secs();
                    if last_access_time > config_data.idle_in_sleep_time as u64 {
                        operating_mode = false;
                        info!("Idle time {:?} over. Go to sleep", last_access_time);
                        emmc_cam_power.set_high().expect("Set emmc_cam_power high failure");
                        deep_and_light_sleep_start(SleepMode::SleepModeDeep, 0);
                    }
                }
            }    
        }

        let key_event = touchpad.get_key_event_and_clear();
        for key in key_event {
            match key {
                KeyEvent::CenterKeyUp => {
                    // millisecond
                    let push_time = touchpad.get_button_press_time(Key::Center);
                    if push_time > 3000 {
                        info!("Long press center key {}", push_time);
                        server_info.capture_started = true;
                        server_info.capture_frames_at_once = -1;
                        movie_mode = true;
                        server.as_mut().unwrap().set_server_capture_started(server_info.capture_started);
                        server.as_mut().unwrap().set_capture_frames_at_once(server_info.capture_frames_at_once);

                    }
                    else {
                        info!("Center key down");
                        server_info.capture_started = false;
                        server_info.capture_frames_at_once = 0;
                        movie_mode = false;
                        // indicator off
                        // led_ind.set_high().expect("Set indicator high failure");
                        server.as_mut().unwrap().set_server_capture_started(server_info.capture_started);
                        server.as_mut().unwrap().set_capture_frames_at_once(server_info.capture_frames_at_once);
                    }
                    if server_info.duration == 0 && server_info.leap_time.day < 0
                        && server_info.leap_time.hour < 0 && server_info.leap_time.minute < 0 {
                        server_info.duration = 90;
                    }                
                },
                _ => {
                }
            }
        }
        if operating_mode && one_shot {
            server_info.capture_started = true;
            server.as_mut().unwrap().set_capture_frames_at_once(0);
            capture.set_overwrite_saved(false);
        }
    
        if server_info.resolution != current_resolution {
            info!("Resolution changed: {} -> {}", current_resolution, server_info.resolution);
            current_resolution = server_info.resolution;
            capture.change_resolution(current_resolution);
        }
        capture.set_capturing_duration(server_info.capture_frames_at_once);
        let mut tempval : f32 = 0.0;
        unsafe {
            esp_idf_svc::hal::sys::temperature_sensor_get_celsius(&mut *temp_sensor_ptr, &mut tempval);
            server.as_mut().unwrap().set_temperature(tempval);
        }

        if server_info.capture_started {
            log::info!("System Temperature: {:.2}°C", tempval);    
            if !capture_indicator_on {
                // indicator on
                // led_ind.set_low().expect("Set indicator low failure");
                capture_indicator_on = true;
            }
            else {
                // led_ind.set_high().expect("Set indicator high failure");
                capture_indicator_on = false;
            }
            if current_track_id != server_info.track_id {
                info!("Track ID changed: {} -> {}", current_track_id, server_info.track_id); 
                current_track_id = server_info.track_id;
                capture_id = 0;
            }
            if server_info.autofocus_once {
                if capture_id == 0 {
                    capture.autofocus_request();
                }
            }
            else {
                capture.autofocus_request();
            }
            if capture_id == 0 {
                next_capture_time = SystemTime::now();
                capture.set_overwrite_saved(server_info.overwrite_saved);
            }
            else {
                capture.set_overwrite_saved(false);
            }
            if movie_mode && capture_id == 0 || !movie_mode {
                // indicator on
                // led_ind.set_low().expect("Set indicator low failure");
                info!("Capture Started Track ID: {} Count: {} Resolution: {}", current_track_id, capture_id, current_resolution);
                capture.capture_request(current_track_id, capture_id);
            }
            if movie_mode {
                capture_id += 1;
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            loop {
                if capture.get_capture_status() {
                    info!("Capture done");
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            // get last capture id
            capture_id = capture.get_capture_id();
            if server_info.query_openai {
                info!("Query OpenAI: Track :{} frame No.:{}", current_track_id, capture_id);
                monitoring_thread.set_query_start(server_info.query_prompt.clone(), current_track_id, capture_id);
                loop {
                    if !monitoring_thread.get_query_status() {
                        let reply = monitoring_thread.get_query_reply();
                        info!("Query reply: {}", reply);
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                if monitoring_thread.get_posted_status() {
                    server_info.last_posted_date_time = SystemTime::now();
                    server.as_mut().unwrap().set_last_posted_date_time(server_info.last_posted_date_time);
                    monitoring_thread.set_last_posted_date_time(server_info.last_posted_date_time, server_info.post_interval);
                }
            }
            let capture_info = capture.get_capture_info();
            if capture_info.status {
                info!("Write Frame ID {}: width:{} height:{} image_size:{}", capture_id, capture_info.width, capture_info.height, capture_info.size);
                if server_info.status_report && (last_status_posted_time.elapsed().unwrap().as_secs() > server_info.status_report_interval as u64) {
                    // capture time
                    let dt_utc : DateTime<Utc> = DateTime::<Utc>::from(SystemTime::now());
                    let fixed_offset = FixedOffset::east_opt(server_info.timezone * 3600).unwrap();
                    let dt_local = DateTime::<Local>::from_naive_utc_and_offset(dt_utc.naive_utc(), fixed_offset);
                    let capture_time = dt_local.format("%Y-%m-%d %H:%M:%S").to_string();
                    let message = format!("STATUS REPORT: {}:{} {} {}V {}dBm", 
                        current_track_id, capture_id, capture_time,
                         battery_voltage, wifi::get_rssi());
                    monitoring_thread.post_message_request(message, current_track_id, capture_id);
                    loop {
                        if !monitoring_thread.get_post_message_status() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    last_status_posted_time = SystemTime::now();
                }    
                server_info.last_capture_date_time = SystemTime::now();
                if server_enalbed {
                    server.as_mut().unwrap().set_last_capture_date_time(server_info.last_capture_date_time);
                }
                server_info.current_capture_id = capture_id;
                // increment capture_id
                capture_id += 1;
            }

            // indicator off
            // led_ind.set_high().expect("Set indicator high failure");
            capture_indicator_on = false;            
            
            if one_shot {
                server_info.capture_started = false;
                capture_id = 0;
                server.as_mut().unwrap().set_one_shot_completed();
                thread::sleep(Duration::from_millis(100));
                continue;
            }

            next_capture_time = match get_next_wake_time(
                    server_info.leap_time,
                    server_info.timezone,
                    next_capture_time,
                    server_info.duration,
                    server_info.capture_start_time,
                    server_info.capture_end_time) {
                Some(time) => time,
                None => {
                    info!("Capture end");
                    server_info.capture_started = false;
                    if server_enalbed {
                        server.as_mut().unwrap().set_server_capture_started(server_info.capture_started);
                    }
                    capture_id = 0;
                    unsafe {
                        IMAGE_COUNT_ID = capture_id;
                        DEEP_SLEEP_AUTO_CAPTURE = server_info.capture_started;
                        DURATION_TIME = current_duration;
                        CURRENT_RESOLUTION = current_resolution;
                        CURRENT_TRACK_ID = current_track_id;
                        LAST_CAPTURE_TIME = server_info.last_capture_date_time.duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;
                        LAST_POSTED_TIME = server_info.last_posted_date_time.duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;
                        CAPTURE_START_TIME = server_info.capture_start_time.duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;
                        CAPTURE_END_TIME = server_info.capture_end_time.duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;
                        LAST_STATUS_POSTED_TIME = last_status_posted_time.duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;
                    }
                    emmc_cam_power.set_high().expect("Set emmc_cam_power high failure");
                    deep_and_light_sleep_start(SleepMode::SleepModeDeep, 0);
                    SystemTime::now() // not reached
                }
            };
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
                        IMAGE_COUNT_ID = capture_id;
                        DEEP_SLEEP_AUTO_CAPTURE = true;
                        NEXT_CAPTURE_TIME = next_capture_time.duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;
                        DURATION_TIME = current_duration;
                        CURRENT_RESOLUTION = current_resolution;
                        CURRENT_TRACK_ID = current_track_id;
                        LAST_CAPTURE_TIME = server_info.last_capture_date_time.duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;
                        LAST_POSTED_TIME = server_info.last_posted_date_time.duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;
                        CAPTURE_START_TIME = server_info.capture_start_time.duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;
                        CAPTURE_END_TIME = server_info.capture_end_time.duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;
                        LAST_STATUS_POSTED_TIME = last_status_posted_time.duration_since(UNIX_EPOCH).unwrap().as_secs() as u64;
                    }
                    emmc_cam_power.set_high().expect("Set emmc_cam_power high failure");
                    deep_and_light_sleep_start(SleepMode::SleepModeDeep, sleep_time.as_secs());
                }
            }
        }
        else {
            if capture_id > 0 {
                capture_id = 0;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

// duration: > 0: Capture every duration seconds, = 0: Capture at specific time by LeapTime
fn get_next_wake_time(lt: LeapTime, timezone: i32, mut next_capture_time: SystemTime, duration: u32,
    capture_start_time: SystemTime, capture_end_time: SystemTime ) -> Option<SystemTime> {
    let now = SystemTime::now();
    let start_time_utc_str = DateTime::<Utc>::from(capture_start_time).format("%Y-%m-%d %H:%M:%S").to_string();
    let end_time_utc_str = DateTime::<Utc>::from(capture_end_time).format("%Y-%m-%d %H:%M:%S").to_string();
    let now_utc_str = DateTime::<Utc>::from(now).format("%Y-%m-%d %H:%M:%S").to_string();
    info!("Start {} - {} Now: {}", start_time_utc_str, end_time_utc_str, now_utc_str);    
    if duration > 0 {
        next_capture_time = next_capture_time + Duration::from_secs(duration as u64);
        if capture_start_time < capture_end_time {
            if next_capture_time < capture_start_time {
                next_capture_time = capture_start_time;
            }
            else if next_capture_time > capture_end_time {
                // stop capture
                return None;
            }
        }
        return Some(next_capture_time);
    }
    else {
        let offset = FixedOffset::east_opt(timezone * 60 * 60).unwrap();
        let now_with_offset = Local::now().with_timezone(&offset);
        // parse now_with_offset to string
        let now_str = now_with_offset.format("%Y-%m-%d %H:%M:%S").to_string();
        info!("Now with offset: {}", now_str);
        // parse now_str to year, day, month, hour, minute, second
        let now_year = now_with_offset.year();
        let now_month = now_with_offset.month();
        let now_day = now_with_offset.day();
        let now_hour = now_with_offset.hour();
        let now_naive = now_with_offset.naive_local();
        let mut lt_hour = lt.hour;
        let mut lt_minute = lt.minute;
        let lt_day = lt.day;
        if lt_day > 0 {
            // LeapTime is specific day
            if lt_hour < 0 { lt_hour = 0; }
            if lt_minute < 0 { lt_minute = 0; }
            let leap_time = NaiveDate::from_ymd_opt(now_year, now_month, lt_day as u32).unwrap()
                .and_hms_opt(lt_hour as u32, lt_minute as u32, 0).unwrap().and_local_timezone(offset).unwrap();
            if now_naive < leap_time.naive_local() {
                // convert leap_time to UTC as DateTime
                next_capture_time = leap_time.into();
            }
            else {
                // LeapTime is interval time, get duration from now to LeapTime
                let mut next_yaer = now_year;
                let mut next_month = now_month;
                if now_month == 12 {
                    next_yaer += 1;
                    next_month = 1;
                }
                else {
                    next_month += 1;
                }
                let today_leap_time = NaiveDate::from_ymd_opt(now_year, now_month, lt_day as u32)
                    .unwrap().and_hms_opt(lt_hour as u32, lt_minute as u32, 0).unwrap().and_local_timezone(offset).unwrap();
                if now_naive < today_leap_time.naive_local() {
                    // today
                    next_capture_time = today_leap_time.into();
                }
                else {
                    // next month
                    let next_month_leap_time = NaiveDate::from_ymd_opt(next_yaer, next_month, lt_day as u32)
                        .unwrap().and_hms_opt(lt_hour as u32, lt_minute as u32, 0).unwrap().and_local_timezone(offset).unwrap();
                    next_capture_time = next_month_leap_time.into();
                }
            }
        }
        else {
            if lt.hour >= 0 {
                // LeapTime is specific time
                if lt_minute < 0 { lt_minute = 0; }
                let leap_time = NaiveDate::from_ymd_opt(now_year, now_month, now_day)
                    .unwrap().and_hms_opt(lt_hour as u32, lt_minute as u32, 0).unwrap().and_local_timezone(offset).unwrap();
                if now_naive < leap_time.naive_local() {
                    // convert leap_time to UTC as DateTime
                    next_capture_time = leap_time.into();
                }
                else {
                    // LeapTime is interval time, get duration from now to LeapTime
                    let tomorrow = now_naive.date() + ChronoDuration::days(1);
                    let tomorrow_leap_time = NaiveDate::from_ymd_opt(tomorrow.year(), tomorrow.month(), tomorrow.day())
                        .unwrap().and_hms_opt(lt_hour as u32, lt_minute as u32, 0).unwrap().and_local_timezone(offset).unwrap();
                    next_capture_time = tomorrow_leap_time.into();
                }
            }
            else {
                if lt.minute > 0 {
                    // LeapTime is specific time 
                    let leap_time = NaiveDate::from_ymd_opt(now_year, now_month, now_day)
                        .unwrap().and_hms_opt(now_hour, lt_minute as u32, 0).unwrap().and_local_timezone(offset).unwrap();
                    if now_naive < leap_time.naive_local() {
                        // convert leap_time to UTC as DateTime
                        next_capture_time = leap_time.into();
                    }
                    else {
                        // LeapTime is interval time, get duration from now to LeapTime
                        if now_hour == 23 {
                            let tomorrow = now_naive.date() + ChronoDuration::days(1);
                            let tomorrow_leap_time = NaiveDate::from_ymd_opt(tomorrow.year(), tomorrow.month(), tomorrow.day())
                                .unwrap().and_hms_opt(0, lt_minute as u32, 0).unwrap().and_local_timezone(offset).unwrap();
                            next_capture_time = tomorrow_leap_time.into();
                        }
                        else {
                            let next_hour = now_hour + 1;
                            let next_leap_time = NaiveDate::from_ymd_opt(now_year, now_month, now_day)
                                .unwrap().and_hms_opt(next_hour, lt_minute as u32, 0).unwrap().and_local_timezone(offset).unwrap();
                            next_capture_time = next_leap_time.into();
                        }
                    }
                }
            }
        }
        if next_capture_time < capture_start_time {
            next_capture_time = capture_start_time;
        }
        else if next_capture_time > capture_end_time && capture_end_time > now.into() {
            // stop capture
            return None;
        }
        return Some(next_capture_time);
    }
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