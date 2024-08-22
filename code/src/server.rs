#[allow(dead_code)]

use log::info;
use embedded_svc::http::Method;
use esp_idf_svc::http::server::{Configuration as HttpServerConfig, EspHttpServer};
use esp_idf_svc::io::EspIOError;
use esp_idf_sys::camera;
use embedded_svc::http::Headers;
use esp_idf_hal::io::{Write, Read};
use std::path::Path;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use chrono::{DateTime, Local, FixedOffset};
use url;
use serde_json;

use base64::prelude::*;
use crate::imagefiles::{ImageFiles, OpenMode};

const MAX_LEN: usize = 1024;

const ACCEPTABLE_RESOLUTIONS: [(&'static str, u32); 14] = [
    ("QVGA",    camera::framesize_t_FRAMESIZE_QVGA),    // 320x240
    ("CIF",     camera::framesize_t_FRAMESIZE_CIF),     // 400x296
    ("HVGA",    camera::framesize_t_FRAMESIZE_HVGA),    // 480x320
    ("VGA",     camera::framesize_t_FRAMESIZE_VGA),     // 640x480
    ("SVGA",    camera::framesize_t_FRAMESIZE_SVGA),    // 800x600
    ("XGA",     camera::framesize_t_FRAMESIZE_XGA),     // 1024x768
    ("HD",      camera::framesize_t_FRAMESIZE_HD),      // 1280x720
    ("SXGA",    camera::framesize_t_FRAMESIZE_SXGA),    // 1280x1024
    ("UXGA",    camera::framesize_t_FRAMESIZE_UXGA),    // 1600x1200
    ("FHD",     camera::framesize_t_FRAMESIZE_FHD),     // 1920x1080
    ("QXGA",    camera::framesize_t_FRAMESIZE_QXGA),    // 2048x1536
    ("QSXGA",   camera::framesize_t_FRAMESIZE_QSXGA),   // 2592x1944
    ("WQXGA",   camera::framesize_t_FRAMESIZE_WQXGA),   // 2560x1600
    ("QHD",     camera::framesize_t_FRAMESIZE_QHD),     // 2560x1440
];

#[derive(Debug, Clone, Copy)]
pub struct LeapTime {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
}

#[derive(Debug, Clone)]
pub struct ControlServerInfo {
    pub need_to_save: bool,
    pub capture_started: bool,
    pub resolution: u32,
    pub track_id: u32,
    pub duration: u32,
    pub leap_time: LeapTime,
    pub timezone: i32,
    pub idle_in_sleep_time: u32,
    pub auto_capture: bool,
    pub last_access_time: SystemTime,
    pub query_openai: bool,
    pub query_prompt: String,
    pub openai_model: String,
    pub rssi: i32,
    pub battery_voltage: f32,
    pub current_capture_id: u32,
    pub last_capture_date_time: SystemTime,
    pub one_shot: bool,
    pub one_shot_completed: bool,
    pub autofocus_once: bool,
    pub status_report: bool,
    pub status_report_interval: u32,
    pub last_posted_date_time: SystemTime,
    pub post_interval: u32,
    pub capture_start_time: SystemTime,
    pub capture_end_time: SystemTime,
    pub capture_frames_at_once: i32,
    pub overwrite_saved: bool,
    pub temperature: f32,
    pub direct_write_mode: bool,
    pub jpeg_quality: u32,
}

impl ControlServerInfo {
    pub fn new() -> ControlServerInfo {
        let now = SystemTime::now();
        ControlServerInfo {
            need_to_save: false,
            capture_started: false,
            track_id: 0,
            duration: 90,
            leap_time: LeapTime {
                year: -1,
                month: -1,
                day: -1,
                hour: -1,
                minute: -1,
                second: -1,
            },
            timezone: 9,
            resolution: camera::framesize_t_FRAMESIZE_VGA,
            idle_in_sleep_time: 300,
            auto_capture: false,
            last_access_time: now,
            query_openai: false,
            query_prompt: String::from(""),
            openai_model: String::from(""),
            rssi: 0,
            battery_voltage: 0.0,
            current_capture_id: 0,
            last_capture_date_time: now,
            one_shot: false,
            one_shot_completed: false,
            autofocus_once: false,
            status_report: false,
            status_report_interval: 3600,
            last_posted_date_time: now,
            post_interval: 3600,
            capture_start_time: now,
            capture_end_time: now,
            capture_frames_at_once: 0,
            overwrite_saved: false,
            temperature: 0.0,
            direct_write_mode: false,
            jpeg_quality: 12,
        }
    }    
}

pub struct ControlServer {
    http_server: EspHttpServer<'static>,
    server_info: Arc<Mutex<ControlServerInfo>>,
}

impl ControlServer {
    pub fn new(info: &ControlServerInfo) -> Result<ControlServer, EspIOError> {
        let http_server = EspHttpServer::new(&HttpServerConfig::default())?;
        Ok(ControlServer { http_server,
                           server_info: Arc::new(Mutex::new(info.clone()))})
    }

    pub fn start(&mut self) {
        let server_info_start = self.server_info.clone();
        // start capture by POST method {"request": "start" or "stop"}
        self.http_server.fn_handler("/capture", Method::Post, move |mut request| {
            let server_info = server_info_start.clone();
            let len = request.content_len().unwrap_or(0) as usize;
            let mut server_info = server_info.lock().unwrap();
            if len > MAX_LEN {
                request.into_status_response(413)?
                    .write_all("Request too big".as_bytes())?;
                return Ok::<(), EspIOError>(());
            }
            let mut body = vec![0; len];
            match request.read_exact(&mut body) {
                Ok(_) => (),
                Err(_e) => {
                    request.into_status_response(500)?
                        .write_all("Failed to read body".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            }
            let body = match std::str::from_utf8(&body) {
                Ok(body) => body,
                Err(_e) => {
                    request.into_status_response(400)?
                        .write_all("Invalid body".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            info!("Body: {:?}", body);
            let json: serde_json::Value = match serde_json::from_str(body) {
                Ok(json) => json,
                Err(e) => {
                    info!("Failed to parse JSON: {:?}", e);
                    request.into_status_response(400)?
                        .write_all("Invalid JSON".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            // get request
            let request_param = match json["request"].as_str() {
                Some(request_param) => request_param,
                None => {
                    request.into_status_response(400)?
                        .write_all("No request".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            match request_param {
                "start" => {
                    server_info.capture_started = true;
                }
                "stop" => {
                    server_info.capture_started = false;
                }
                _ => {
                    request.into_status_response(400)?
                        .write_all("Invalid request".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            }
            if server_info.capture_started {
                // get track_id
                let track_id = match json["trackid"].as_u64() {
                    Some(track_id) => track_id as u32,
                    None => {
                        0
                    }
                };
                server_info.track_id = track_id;
                //  get duration
                let duration = match json["duration"].as_u64() {
                    Some(duration) => duration as u32,
                    None => {
                        10
                    }
                };
                server_info.duration = duration;
                // get resolution
                let resolution = match json["resolution"].as_str() {
                    Some(resolution) => resolution,
                    None => {
                        "VGA"
                    }
                };
                server_info.resolution = ACCEPTABLE_RESOLUTIONS.iter()
                    .find(|(name, _)| name == &resolution)
                    .map(|(_, value)| *value)
                    .unwrap_or(camera::framesize_t_FRAMESIZE_VGA);

                // get leap_time
                let leap_time = match json["leaptime"].as_object() {
                    Some(leap_time) => {
                        // let year = match leap_time.get("year") {
                        //     Some(year) => year.as_i64().unwrap(),
                        //     None => 0,
                        // };
                        // let month = match leap_time.get("month") {
                        //     Some(month) => month.as_u64().unwrap(),
                        //     None => 1,
                        // };
                        let year = 0;
                        let month = 0;
                        let day = match leap_time.get("day") {
                            Some(day) => day.as_i64().unwrap(),
                            None => 1,
                        };
                        let hour = match leap_time.get("hour") {
                            Some(hour) => hour.as_i64().unwrap(),
                            None => 0,
                        };
                        let minute = match leap_time.get("minute") {
                            Some(minute) => minute.as_i64().unwrap(),
                            None => 0,
                        };
                        let second = match leap_time.get("second") {
                            Some(second) => second.as_i64().unwrap(),
                            None => 0,
                        };
                        LeapTime {
                            year: year as i32,
                            month: month as i32,
                            day: day as i32,
                            hour: hour as i32,
                            minute: minute as i32,
                            second: second as i32,
                        }
                    }
                    None => {
                        LeapTime {
                            year: -1,
                            month: -1,
                            day: -1,
                            hour: -1,
                            minute: -1,
                            second: -1,
                        }
                    }
                };
                info!("Leap Time: {:?}", leap_time);
                server_info.leap_time = leap_time;
                // get capture start date & time
                let default_capture_start_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() as i64;
                let capture_start_time = match json["captureStartTime"].as_str() {
                    Some(capture_start_time) => {
                        let capture_start_time = &format!("{}:00{}{:02}:00", capture_start_time, if server_info.timezone >= 0 {"+"} else {"-"},  server_info.timezone);
                        info!("Capture Start Time: {}", capture_start_time);
                        let parse_capture_start_time = chrono::DateTime::parse_from_rfc3339(capture_start_time);
                        let capture_start_time = match parse_capture_start_time {
                            Ok(capture_start_time) => {
                                let capture_start_time = capture_start_time.with_timezone(&chrono::Utc);
                                let capture_start_time = capture_start_time.timestamp();
                                capture_start_time
                            }
                            Err(e) => {
                                info!("Failed to parse capture start time: {:?}", e);
                                default_capture_start_time
                            }
                        };
                        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(capture_start_time as u64)
                    }
                    None => {
                        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(default_capture_start_time as u64)
                    }
                };
                // transfer capture_start_time to Time struct format
                server_info.capture_start_time = capture_start_time;
                // get capture end date & time
                let capture_end_time = match json["captureEndTime"].as_str() {
                    Some(capture_end_time) => {
                        //2024-01-01T00:00:00 LOCAL TIME to UTC
                        let capture_end_time = &format!("{}:00{}{:02}:00", capture_end_time, if server_info.timezone >= 0 {"+"} else {"-"}, server_info.timezone);
                        info!("Capture End Time: {}", capture_end_time);
                        let parse_capture_end_time = chrono::DateTime::parse_from_rfc3339(capture_end_time);
                        let capture_end_time = match parse_capture_end_time {
                            Ok(capture_end_time) => {
                                let capture_end_time = capture_end_time.with_timezone(&chrono::Utc);
                                let capture_end_time = capture_end_time.timestamp();
                                capture_end_time
                            }
                            Err(e) => {
                                info!("Failed to parse capture end time: {:?}", e);
                                default_capture_start_time
                            }
                        };
                        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(capture_end_time as u64)
                    }
                    None => {
                        capture_start_time
                    }
                };
                // transfer capture_end_time to Time struct format
                server_info.capture_end_time = capture_end_time;
                // get capture frames at once
                let capture_frames_at_once = match json["captureFramesAtOnce"].as_i64() {
                    Some(capture_frames_at_once) => capture_frames_at_once as i32,
                    None => {
                        1
                    }
                };
                server_info.capture_frames_at_once = capture_frames_at_once;
                // get overwrite saved
                let overwrite_saved = match json["overwriteSaved"].as_bool() {
                    Some(overwrite_saved) => overwrite_saved,
                    None => {
                        false
                    }
                };
                server_info.overwrite_saved = overwrite_saved;
                server_info.need_to_save = true;
            }
            let response = request.into_ok_response();
            let status = if server_info.capture_started {
                "Capture started"
            } else {
                "Capture stopped"
            };
            server_info.last_access_time = SystemTime::now();
            let status_json = format!("{{\"status\": \"{}\"}}", status);
            response?.write_all(status_json.as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();

        // get capture status by GET method {"status": "Capture started" or "Capture stopped"}
        let server_info_status = self.server_info.clone();
        self.http_server.fn_handler("/capture", Method::Get, move |request| {
            let server_info = server_info_status.clone();
            let response = request.into_ok_response();
            let server_info = server_info.lock().unwrap();
            let status = if server_info.capture_started {
                "Capture started"
            } else {
                "Capture stopped"
            };
            let status_json = format!("{{\"status\": \"{}\"}}", status);
            response?.write_all(status_json.as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();

        // oneshot capture by POST method {"trackid": 1}
        let server_info_capture = self.server_info.clone();
        self.http_server.fn_handler("/oneshot", Method::Post, move |mut request| {
            let server_info_clone = server_info_capture.clone();
            let len = request.content_len().unwrap_or(0) as usize;
            let mut server_info = server_info_clone.lock().unwrap();
            if len > MAX_LEN {
                request.into_status_response(413)?
                    .write_all("Request too big".as_bytes())?;
                return Ok::<(), EspIOError>(());
            }
            let mut body = vec![0; len];
            match request.read_exact(&mut body) {
                Ok(_) => (),
                Err(_e) => {
                    request.into_status_response(500)?
                        .write_all("Failed to read body".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            }
            let body = match std::str::from_utf8(&body) {
                Ok(body) => body,
                Err(_e) => {
                    request.into_status_response(400)?
                        .write_all("Invalid body".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            let json: serde_json::Value = match serde_json::from_str(body) {
                Ok(json) => json,
                Err(_e) => {
                    request.into_status_response(400)?
                        .write_all("Invalid JSON".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            let track_id = match json["trackid"].as_u64() {
                Some(track_id) => track_id as u32,
                None => {
                    0
                }
            };
            server_info.track_id = track_id;
            server_info.one_shot = true;
            server_info.last_access_time = SystemTime::now();
            let response = request.into_ok_response();
            response?.write_all("Oneshot capture started".as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();

        // get oneshot completion status by GET method {"status": true or false }
        let server_info_status = self.server_info.clone();
        self.http_server.fn_handler("/oneshot", Method::Get, move |request| {
            let server_info = server_info_status.clone();
            let response = request.into_ok_response();
            let server_info = server_info.lock().unwrap();
            let status = if server_info.one_shot_completed {
                "true"
            } else {
                "false"
            };
            let status_json = format!("{{\"status\": {}}}", status);
            response?.write_all(status_json.as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();

        // set resolution by POST method {"resolution": "VGA"}        
        let server_info_resolution = self.server_info.clone();
        self.http_server.fn_handler("/resolution", Method::Post, move |mut request| {
            let server_info = server_info_resolution.clone();
            let len = request.content_len().unwrap_or(0) as usize;
            let mut server_info = server_info.lock().unwrap();
            if len > MAX_LEN {
                request.into_status_response(413)?
                    .write_all("Request too big".as_bytes())?;
                return Ok(());
            }
            let mut body = vec![0; len];
            match request.read_exact(&mut body) {
                Ok(_) => (),
                Err(_e) => {
                    request.into_status_response(500)?
                        .write_all("Failed to read body".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            }
            let body = match std::str::from_utf8(&body) {
                Ok(body) => body,
                Err(_e) => {
                    request.into_status_response(400)?
                        .write_all("Invalid body".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            let json: serde_json::Value = match serde_json::from_str(body) {
                Ok(json) => json,
                Err(_e) => {
                    request.into_status_response(400)?
                        .write_all("Invalid JSON".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            let resolution = match json["resolution"].as_str() {
                Some(resolution) => resolution,
                None => {
                    request.into_status_response(400)?
                        .write_all("No resolution".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            // get resolution value if resolution is acceptable
            let resolution_value = ACCEPTABLE_RESOLUTIONS.iter()
                .find(|(name, _)| name == &resolution)
                .map(|(_, value)| *value);
            match resolution_value {
                Some(resolution_value) => {
                    server_info.resolution = resolution_value;
                    server_info.last_access_time = SystemTime::now();
                }
                None => {
                    request.into_status_response(400)?
                        .write_all("Invalid resolution".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            }
            let response = request.into_ok_response();
            response?.write_all("Resolution set".as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();

        // get resolution by GET method {"resolution": "VGA"}
        let server_info_resolution = self.server_info.clone();
        self.http_server.fn_handler("/resolution", Method::Get, move |request| {
            let server_info = server_info_resolution.clone();
            let response = request.into_ok_response();
            let server_info = server_info.lock().unwrap();
            let resolution = server_info.resolution.clone();
            let resolution_name = ACCEPTABLE_RESOLUTIONS.iter()
                .find(|(_, value)| value == &resolution)
                .map(|(name, _)| name);
            match resolution_name {
                Some(resolution_name) => {
                    let resolution_json = format!("{{\"resolution\": \"{}\"}}", resolution_name);
                    response?.write_all(resolution_json.as_bytes())?;
                }
                None => {
                    response?.write_all("Unknown resolution".as_bytes())?;
                }
            }
            Ok::<(), EspIOError>(())
        }).unwrap();

        // get image by GET method /data?trackid=1&fromframe=0&toframe=10
        let server_info_get_image = self.server_info.clone();
        self.http_server.fn_handler("/data", Method::Get, move |request| {
            // read all request uri
            let uri = request.uri();
            // info!("URI: {:?}", uri);
            let uri_str = format!("http://localhost{}", uri);
            let parsed_uri = url::Url::parse(&uri_str);
            let args = match parsed_uri {
                Ok(parsed_uri) => {
                    let args = parsed_uri.query_pairs()
                    .map(|(key, value)| (key.into_owned(), value.into_owned()))
                    .collect::<HashMap<String, String>>();
                    info!("Args: {:?}", args);        
                    // info!("Parsed URI: {:?}", parsed_uri);
                    args
                }
                Err(e) => {
                    info!("Failed to parse URI: {:?}", e);
                    HashMap::new()
                }
            };
            // get trace_id
            let trackid = || -> Option<u32> {
                for (key, value) in &args {
                    if key == "trackid" {
                        return Some(value.parse().unwrap());
                    }
                }
                None
            };
            if trackid().is_none() {
                info!("trackid not found");
                let response = request.into_ok_response();
                response?.write_all("No frame".as_bytes())?;
                return Ok::<(), EspIOError>(());
            }
            // get fromframe
            let fromframe : i32 = {
                let mut intval = 0;
                for (key, value) in &args {
                    if key == "fromframe" {
                        intval = value.parse().unwrap();
                        break;
                    }
                }
                intval
            };
            // get toframe
            let mut toframe : i32 = {
                let mut intval = -1;
                for (key, value) in &args {
                    if key == "toframe" {
                        intval = value.parse().unwrap();
                        break;
                    }
                }
                intval
            };
            if toframe > 0 && fromframe > toframe {
                toframe = fromframe;
            }
            let headers = [
                ("Content-Type", "multipart/x-mixed-replace; boundary=--timeleapcamboundary"),
            ];
            let server_info_clone = server_info_get_image.clone();
            let file_path = format!("/eMMC/T{}/capture.dat", trackid().unwrap());
            let mut r_image = match ImageFiles::new(Path::new(&file_path), OpenMode::Read) {
                Ok(r_image) => r_image,
                Err(e) => {
                    info!("Failed to open file: {:?} {:?}", file_path, e);
                    let response = request.into_ok_response();
                    response?.write_all("No Capture Data".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            let mut count = match fromframe {
                // last image
                -1 => (r_image.get_nof_images()  - 1) as i32,
                _ => fromframe,
            };
            match r_image.seek_image(count as u32){
                Ok(_) => (),
                Err(e) => {
                    info!("Not found image: {:?}", e);
                    let response = request.into_ok_response();
                    response?.write_all("No Capture Data".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            }
            let mut response = request.into_response(200, Some("OK"), &headers).unwrap();
            loop {    
                // let get_time = SystemTime::now();
                let buffer = match r_image.read_image(){
                    Ok(buffer) => buffer,
                    Err(e) => {
                        info!("Failed to read image: {:?}", e);
                        break;
                    }
                };
                let read_size = buffer.len();
                // let get_elapsed = get_time.elapsed().unwrap().as_millis();
                // info!("Read image: {} bytes, elapsed: {} ms", read_size, get_elapsed);
                // let send_time = SystemTime::now();
                response.write_all("--timeleapcamboundary\r\n".as_bytes())?;
                response.write_all("Content-Type: image/jpeg\r\n".as_bytes())?;
                let context_length = format!("Content-Length: {}\r\n\r\n", read_size);
                response.write_all(context_length.as_bytes())?;
                response.write_all(&buffer)?;
                response.write_all("\r\n".as_bytes())?;
                // let send_elapsed = send_time.elapsed().unwrap().as_millis();
                // info!("Send image: {} bytes, elapsed: {} ms", read_size, send_elapsed);
                count += 1;
                let mut server_info = server_info_clone.lock().unwrap();
                server_info.last_access_time = SystemTime::now();
                drop(server_info);    
                if toframe >= 0 && count > toframe {
                    break;
                }
            }
            Ok::<(), EspIOError>(())
        }).unwrap();

        // get image by GET method /data?trackid=1&fromframe=0&toframe=10
        let server_info_get_image = self.server_info.clone();
        self.http_server.fn_handler("/images", Method::Get, move |request| {
            // read all request uri
            let uri = request.uri();
            // info!("URI: {:?}", uri);
            let uri_str = format!("http://localhost{}", uri);
            let parsed_uri = url::Url::parse(&uri_str);
            let args = match parsed_uri {
                Ok(parsed_uri) => {
                    let args = parsed_uri.query_pairs()
                    .map(|(key, value)| (key.into_owned(), value.into_owned()))
                    .collect::<HashMap<String, String>>();
                    info!("Args: {:?}", args);        
                    // info!("Parsed URI: {:?}", parsed_uri);
                    args
                }
                Err(e) => {
                    info!("Failed to parse URI: {:?}", e);
                    HashMap::new()
                }
            };
            // get trace_id
            let trackid = || -> Option<u32> {
                for (key, value) in &args {
                    if key == "trackid" {
                        return Some(value.parse().unwrap());
                    }
                }
                None
            };
            if trackid().is_none() {
                info!("trackid not found");
                let response = request.into_ok_response();
                response?.write_all("No frame".as_bytes())?;
                return Ok::<(), EspIOError>(());
            }
            // get fromframe
            let fromframe : i32 = {
                let mut intval = 0;
                for (key, value) in &args {
                    if key == "fromframe" {
                        intval = value.parse().unwrap();
                        break;
                    }
                }
                intval
            };
            // get toframe
            let mut toframe : i32 = {
                let mut intval = -1;
                for (key, value) in &args {
                    if key == "toframe" {
                        intval = value.parse().unwrap();
                        break;
                    }
                }
                intval
            };
            if toframe > 0 && fromframe > toframe {
                toframe = fromframe;
            }
            let headers = [
                ("Content-Type", "multipart/x-mixed-replace; boundary=--timeleapcamboundary"),
                ("Content-Disposition", "attachment; filename=\"image.jpeg\""),
            ];
            let mut count = fromframe;
            let server_info_clone = server_info_get_image.clone();
            let file_path = format!("/eMMC/T{}/capture.dat", trackid().unwrap());
            let mut r_image = match ImageFiles::new(Path::new(&file_path), OpenMode::Read) {
                Ok(r_image) => r_image,
                Err(e) => {
                    info!("Failed to open file: {:?} {:?}", file_path, e);
                    let response = request.into_ok_response();
                    response?.write_all("No Capture Data".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            match r_image.seek_image(count as u32){
                Ok(_) => (),
                Err(e) => {
                    info!("Not found image: {:?}", e);
                    let response = request.into_ok_response();
                    response?.write_all("No Capture Data".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            }
            let mut response = request.into_response(200, Some("OK"), &headers).unwrap();
            let track_id = trackid().unwrap();
            loop {
                let buffer = match r_image.read_image(){
                    Ok(buffer) => buffer,
                    Err(e) => {
                        info!("Failed to read image: {:?}", e);
                        break;
                    }
                };
                let read_size = buffer.len();
                response.write_all("--timeleapcamboundary\r\n".as_bytes())?;
                response.write_all("Content-Type: image/jpeg\r\n".as_bytes())?;
                let filename = format!("t{}i{}.jpg", track_id, count);
                response.write_all(format!("Content-Disposition: attachment; filename={}\r\n", filename).as_bytes())?;
                let context_length = format!("Content-Length: {}\r\n\r\n", read_size);
                response.write_all(context_length.as_bytes())?;
                let base64 = BASE64_STANDARD.encode(&buffer);
                response.write_all(base64.as_bytes())?;
                response.write_all("\r\n".as_bytes())?;
                count += 1;
                let mut server_info = server_info_clone.lock().unwrap();
                server_info.last_access_time = SystemTime::now();
                drop(server_info);
                if toframe >= 0 && count > toframe {
                    break;
                }
            }
            Ok::<(), EspIOError>(())
        }).unwrap();

        // index.html by root path
        let server_info_status = self.server_info.clone();
        self.http_server.fn_handler("/", Method::Get, move |request| {
            let response = request.into_ok_response();
            let server_info = server_info_status.clone();
            let server_info = server_info.lock().unwrap();
            let index_html = index_html(server_info.capture_started);
            response?.write_all(index_html.as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();

        // status.html by GET method
        self.http_server.fn_handler("/status.html", Method::Get, move |request| {
            let response = request.into_ok_response();
            let status_html = status_html();
            response?.write_all(status_html.as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();

        // image.html by GET method
        self.http_server.fn_handler("/image.html", Method::Get, move |request| {
            let response = request.into_ok_response();
            let image_html = image_html();
            response?.write_all(image_html.as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();

        // monitor.html by GET method
        self.http_server.fn_handler("/monitor.html", Method::Get, move |request| {
            let response = request.into_ok_response();
            let monitor_html = monitor_html();
            response?.write_all(monitor_html.as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();

        // config.html by GET method
        self.http_server.fn_handler("/config.html", Method::Get, move |request| {
            let response = request.into_ok_response();
            let config_html = config_html();
            response?.write_all(config_html.as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();

        // Monitoring Start/Stop
        let server_info_status = self.server_info.clone();
        self.http_server.fn_handler("/monitor", Method::Post, move |mut request| {
            let server_info = server_info_status.clone();
            let len = request.content_len().unwrap_or(0) as usize;
            let mut server_info = server_info.lock().unwrap();
            if len > MAX_LEN {
                request.into_status_response(413)?
                    .write_all("Request too big".as_bytes())?;
                return Ok::<(), EspIOError>(());
            }
            let mut body = vec![0; len];
            match request.read_exact(&mut body) {
                Ok(_) => (),
                Err(_e) => {
                    request.into_status_response(500)?
                        .write_all("Failed to read body".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            }
            let body = match std::str::from_utf8(&body) {
                Ok(body) => body,
                Err(_e) => {
                    request.into_status_response(400)?
                        .write_all("Invalid body".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            let json: serde_json::Value = match serde_json::from_str(body) {
                Ok(json) => json,
                Err(_e) => {
                    request.into_status_response(400)?
                        .write_all("Invalid JSON".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            info!("json: {:?}", json);
            // set query_openai
            let monitor = match json["queryopenai"] {
                serde_json::Value::Bool(monitor) => monitor,
                _ => {
                    false
                }
            };
            // set prompt value
            let prompt = match json["queryprompt"].as_str() {
                Some(prompt) => prompt,
                None => {
                    ""
                }
            };
            if monitor {
                server_info.query_prompt = prompt.replace("\r", "").replace("\n", "").to_string();
            }
            server_info.query_openai = monitor;
            server_info.last_access_time = SystemTime::now();
            server_info.need_to_save = true;
            let response = request.into_ok_response();
            let status = if server_info.query_openai {
                "Monitoring started"
            } else {
                "Monitoring stopped"
            };
            let status_json = format!("{{\"status\": \"{}\"}}", status);
            response?.write_all(status_json.as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();

        // button state by GET method
        let server_info_status = self.server_info.clone();
        self.http_server.fn_handler("/state", Method::Get, move |request| {
            let response = request.into_ok_response();
            let server_info = server_info_status.clone();
            let server_info = server_info.lock().unwrap();
            // state is capture_started status and rssi, battery_voltage values send as json format
            let fixed_offset = FixedOffset::east_opt(server_info.timezone * 3600).unwrap();
            // last capture date time
            let last_capture_date_time_utc: DateTime<Local> = server_info.last_capture_date_time.into();
            let last_capture_date_time = DateTime::<Local>::from_naive_utc_and_offset(last_capture_date_time_utc.naive_utc(), fixed_offset);
            let lcdt_str = last_capture_date_time.format("%Y-%m-%d %H:%M:%S").to_string();
            // last posted date time
            let last_posted_date_time_utc: DateTime<Local> = server_info.last_posted_date_time.into();
            let last_posted_date_time = DateTime::<Local>::from_naive_utc_and_offset(last_posted_date_time_utc.naive_utc(), fixed_offset);
            let lpdt_str = last_posted_date_time.format("%Y-%m-%d %H:%M:%S").to_string();
            let state_json = format!("{{\"state\": \"{}\", \"rssi\": {}, \"battery_voltage\": {:.2}, \"capture_id\": {}, \"last_capture_date_time\": \"{}\", \"last_posted_date_time\": \"{}\", \"capture_frames_at_once\": {}, \"overwrite_saved\": {}, \"temperature\": {:.2}}}",
                                        if server_info.capture_started {
                                            "start"
                                        } else {
                                            "stop"
                                        },
                                        server_info.rssi,
                                        server_info.battery_voltage,
                                        server_info.current_capture_id,
                                        if server_info.last_capture_date_time == SystemTime::UNIX_EPOCH { "N/A" } else { &lcdt_str },
                                        if server_info.last_posted_date_time == SystemTime::UNIX_EPOCH { "N/A" } else { &lpdt_str },
                                        server_info.capture_frames_at_once,
                                        server_info.overwrite_saved,
                                        server_info.temperature,
                                    );
            response?.write_all(state_json.as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();

        // save configuration by POST method {"resolution": "VGA", "trackid": 1, "duration": 90}
        let server_info_save = self.server_info.clone();
        self.http_server.fn_handler("/config", Method::Post, move |mut request| {
            let server_info = server_info_save.clone();
            let len = request.content_len().unwrap_or(0) as usize;
            let mut server_info = server_info.lock().unwrap();
            if len > MAX_LEN {
                request.into_status_response(413)?
                    .write_all("Request too big".as_bytes())?;
                return Ok::<(), EspIOError>(());
            }
            let mut body = vec![0; len];
            match request.read_exact(&mut body) {
                Ok(_) => (),
                Err(_e) => {
                    request.into_status_response(500)?
                        .write_all("Failed to read body".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            }
            let body = match std::str::from_utf8(&body) {
                Ok(body) => body,
                Err(_e) => {
                    request.into_status_response(400)?
                        .write_all("Invalid body".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            let json: serde_json::Value = match serde_json::from_str(body) {
                Ok(json) => json,
                Err(_e) => {
                    request.into_status_response(400)?
                        .write_all("Invalid JSON".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            let resolution = match json["resolution"].as_str() {
                Some(resolution) => resolution,
                None => {
                    request.into_status_response(400)?
                        .write_all("No resolution".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            };
            // get resolution value if resolution is acceptable
            let resolution_value = ACCEPTABLE_RESOLUTIONS.iter()
                .find(|(name, _)| name == &resolution)
                .map(|(_, value)| *value);
            match resolution_value {
                Some(resolution_value) => {
                    server_info.resolution = resolution_value;
                }
                None => {
                    request.into_status_response(400)?
                        .write_all("Invalid resolution".as_bytes())?;
                    return Ok::<(), EspIOError>(());
                }
            }
            let trackid = match json["trackid"].as_u64() {
                Some(trackid) => trackid as u32,
                None => {
                    0
                }
            };
            server_info.track_id = trackid;
            let duration = match json["duration"].as_u64() {
                Some(duration) => duration as u32,
                None => {
                    0
                }
            };
            server_info.duration = duration;
            // get timezone
            let timezone = match json["timezone"].as_i64() {
                Some(timezone) => timezone as i32,
                None => {
                    9
                }
            };
            server_info.timezone = timezone;
            // get idle_in_sleep_time
            let idlesleep = match json["idlesleep"].as_u64() {
                Some(idlesleep) => idlesleep as u32,
                None => {
                    300
                }
            };
            server_info.idle_in_sleep_time = idlesleep;
            // get autocapture
            let auto_capture = match json["autocapture"].as_str() {
                Some(auto_capture) => {
                    if auto_capture == "true" {
                        true
                    } else {
                        false
                    }  
                }
                None => {
                    false
                }
            };
            server_info.auto_capture = auto_capture;
            // get openai model
            let openai_model = match json["openai_model"].as_str() {
                Some(openai_model) => openai_model,
                None => {
                    ""
                }
            };
            server_info.openai_model = openai_model.to_string();
            // auto focus once
            let autofocus_once = match json["autofocus_once"].as_str() {
                Some(autofocus_once) => {
                    if autofocus_once == "true" {
                        true
                    } else {
                        false
                    }
                }
                None => {
                    false
                }
            };
            server_info.autofocus_once = autofocus_once;
            // status report
            let status_report = match json["status_report"].as_str() {
                Some(status_report) => {
                    if status_report == "true" {
                        true
                    } else {
                        false
                    }
                }
                None => {
                    false
                }
            };
            server_info.status_report = status_report;
            // status report interval
            let status_report_interval = match json["status_report_interval"].as_u64() {
                Some(status_report_interval) => status_report_interval as u32,
                None => {
                    3600
                }
            };
            server_info.status_report_interval = status_report_interval;
            // post interval
            let post_interval = match json["post_interval"].as_u64() {
                Some(post_interval) => post_interval as u32,
                None => {
                    3600
                }
            };
            server_info.post_interval = post_interval;
            // capture frames at once
            let capture_frames_at_once = match json["captureFramesAtOnce"].as_i64() {
                Some(capture_frames_at_once) => capture_frames_at_once as i32,
                None => {
                    0
                }
            };
            server_info.capture_frames_at_once = capture_frames_at_once;
            // jpeg quality
            let jpeg_quality = match json["jpegQuality"].as_u64() {
                Some(jpeg_quality) => jpeg_quality as u32,
                None => {
                    12
                }
            };
            server_info.jpeg_quality = jpeg_quality;
            // get overwrite saved
            let overwrite_saved = match json["overwriteSaved"].as_bool() {
                Some(overwrite_saved) => overwrite_saved,
                None => {
                    false
                }
            };
            server_info.overwrite_saved = overwrite_saved;
            // direct_write_mode
            let direct_write_mode = match json["directWriteMode"].as_bool() {
                Some(direct_write_mode) => direct_write_mode,
                None => {
                    false
                }
            };
            server_info.direct_write_mode = direct_write_mode;
            server_info.need_to_save = true;
            server_info.last_access_time = SystemTime::now();
            let response = request.into_ok_response();
            response?.write_all("Configuration saved".as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();

        // get configuration by GET method {"resolution": "VGA", "trackid": 1, "duration": 90, "timezone": 9, "idlesleep": 300, "autocapture": false}
        let server_info_current_config = self.server_info.clone();
        self.http_server.fn_handler("/config", Method::Get, move |request| {
            let response = request.into_ok_response();
            let server_info = server_info_current_config.clone();
            let server_info = server_info.lock().unwrap();
            let config_json = format!("{{\"resolution\": \"{}\", \"trackid\": {}, \"duration\": {}, \"timezone\": {}, \"idlesleep\": {}, \"autocapture\": {}, \"queryopenai\": {}, \"queryprompt\": \"{}\", \"openai_model\": \"{}\", \"autofocus_once\": {}, \"status_report\": {}, \"status_report_interval\": {}, \"post_interval\": {}, \"leaptime\": {{\"year\": {}, \"month\": {}, \"day\": {}, \"hour\": {}, \"minute\": {} }}, \"captureFramesAtOnce\": {}, \"overwriteSaved\": {}, \"directWriteMode\": {}, \"jpegQuality\": {}}}",
                                      ACCEPTABLE_RESOLUTIONS.iter()
                                      .find(|(_, value)| value == &server_info.resolution)
                                      .map(|(name, _)| *name).unwrap_or("VGA"),
                                      server_info.track_id,
                                      server_info.duration,
                                      server_info.timezone,
                                      server_info.idle_in_sleep_time,
                                      server_info.auto_capture,
                                      server_info.query_openai,
                                      server_info.query_prompt,
                                      server_info.openai_model,
                                      server_info.autofocus_once,
                                      server_info.status_report,
                                      server_info.status_report_interval,
                                      server_info.post_interval,
                                      server_info.leap_time.year,
                                      server_info.leap_time.month,
                                      server_info.leap_time.day,
                                      server_info.leap_time.hour,
                                      server_info.leap_time.minute,
                                      server_info.capture_frames_at_once,
                                      server_info.overwrite_saved,
                                      server_info.direct_write_mode,
                                      server_info.jpeg_quality,
                                    );
            response?.write_all(config_json.as_bytes())?;
            Ok::<(), EspIOError>(())
        }).unwrap();
    }


    #[allow(dead_code)]
    pub fn get_capture_status(&self) -> bool {
        let server_info = self.server_info.lock().unwrap();
        server_info.capture_started
    }

    #[allow(dead_code)]
    pub fn get_resolution(&self) -> u32 {
        let server_info = self.server_info.lock().unwrap();
        server_info.resolution
    }

    pub fn get_server_info(&self) -> ControlServerInfo {
        let server_info = self.server_info.lock().unwrap();
        server_info.clone()
    }

    pub fn set_server_info(&self, new_server_info: ControlServerInfo) {
        let mut server_info = self.server_info.lock().unwrap();
        *server_info = new_server_info;
    }

    pub fn set_server_capture_started(&self, capture_started: bool) {
        let mut server_info = self.server_info.lock().unwrap();
        server_info.capture_started = capture_started;
    }

    pub fn set_current_rssi(&self, rssi: i32) {
        let mut server_info = self.server_info.lock().unwrap();
        server_info.rssi = rssi;
    }

    #[allow(dead_code)]
    pub fn set_current_battery_voltage(&self, battery_voltage: f32) {
        let mut server_info = self.server_info.lock().unwrap();
        server_info.battery_voltage = battery_voltage;
    }

    pub fn get_one_shot(&self) -> bool {
        let mut server_info = self.server_info.lock().unwrap();
        let one_shot = server_info.one_shot;
        if one_shot {
            server_info.one_shot = false;
            server_info.one_shot_completed = false;
        }
        one_shot
    }

    pub fn set_one_shot_completed(&self) {
        let mut server_info = self.server_info.lock().unwrap();
        server_info.one_shot_completed = true;
    }

    pub fn set_last_capture_date_time(&self, last_capture_date_time: SystemTime) {
        let mut server_info = self.server_info.lock().unwrap();
        server_info.last_capture_date_time = last_capture_date_time;
    }

    pub fn set_last_posted_date_time(&self, last_posted_date_time: SystemTime) {
        let mut server_info = self.server_info.lock().unwrap();
        server_info.last_posted_date_time = last_posted_date_time;
    }

    pub fn set_capture_frames_at_once(&self, capture_frames_at_once: i32) {
        let mut server_info = self.server_info.lock().unwrap();
        server_info.capture_frames_at_once = capture_frames_at_once;
    }

    pub fn set_temperature(&self, temperature: f32) {
        let mut server_info = self.server_info.lock().unwrap();
        server_info.temperature = temperature;
    }
}

fn image_html() -> String {
    format!(
        r#"
<!DOCTYPE HTML><html>
<head>
    <title>Time Leap Cam</title>
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <style>
    html {{font-family: Times New Roman; display: inline-block; text-align: center;}}
    body {{max-width: 900px; margin:0px auto; padding-bottom: 25px;}}
    .thumbnail {{ cursor: pointer; width: 332px; margin: 0 auto; text-align: left;}}
    .topnav {{ background-color: #1206d7; overflow: hidden}}
    .topnav a {{ float: left; color: #f2f2f2; text-align: center; padding: 14px 16px; text-decoration: none; font-size: 17px}}
    .topnav a:hover {{ background-color: #ddd; color: black}}
    .topnav a.active {{ background-color: #0dc044; color: white}}
    .left {{ float: left; width: 50%; font-size: 2.0rem; text-align: left;}}
    .clear {{ clear: both;}}
    .btn {{ border: 2px solid black; border-radius: 5px; background-color: white; color: black; padding: 10px 28px; font-size: 16px; cursor: pointer; margin: 8px 4px;}}
    .download {{ border-color: #04AA6D; color: green; }}
    .download:hover {{ background-color: #04AA6D; color: white; }}
    </style>
</head>

<body>
<div class="topnav">
  <a href="/">CAPTURE</a>
  <a class="active" href="image.html">IMAGE</a>
  <a href="monitor.html">MONITORING</a>
  <a href="config.html">CONFIG</a>
  <a href="status.html">STATUS</a>
</div>
<div style="padding:20px;">
<div class="thumbnail">
<div>
<canvas id="canvas0" width="160" height="120" onclick="drawImageOnWindow(0, 0, -1)"></canvas>
<canvas id="canvas1" width="160" height="120" onclick="drawImageOnWindow(1, 0, -1)"></canvas>
</div>
<div>
<canvas id="canvas2" width="160" height="120" onclick="drawImageOnWindow(2, 0, -1)"></canvas>
<canvas id="canvas3" width="160" height="120" onclick="drawImageOnWindow(3, 0, -1)"></canvas>
</div>
<div>
<canvas id="canvas4" width="160" height="120" onclick="drawImageOnWindow(4, 0, -1)"></canvas>
<canvas id="canvas5" width="160" height="120" onclick="drawImageOnWindow(5, 0, -1)"></canvas>
</div>
<div>
<canvas id="canvas6" width="160" height="120" onclick="drawImageOnWindow(6, 0, -1)"></canvas>
<canvas id="canvas7" width="160" height="120" onclick="drawImageOnWindow(7, 0, -1)"></canvas>
</div>
<div>
<canvas id="canvas8" width="160" height="120" onclick="drawImageOnWindow(8, 0, -1)"></canvas>
<canvas id="canvas9" width="160" height="120" onclick="drawImageOnWindow(9, 0, -1)"></canvas>
</div>
<div>
<canvas id="canvas10" width="160" height="120" onclick="drawImageOnWindow(10, 0, -1)"></canvas>
<canvas id="canvas11" width="160" height="120" onclick="drawImageOnWindow(11, 0, -1)"></canvas>
</div></div>

<div class="clear">
<div class="left">
<label for="trackidSelect">Download Track: </label></div>
<div class="left">
<select id="trackidSelect">
<option value="0">0</option>
<option value="1">1</option>
<option value="2">2</option>
<option value="3">3</option>
<option value="4">4</option>
<option value="5">5</option>
<option value="6">6</option>
<option value="7">7</option>
<option value="8">8</option>
<option value="9">9</option>
<option value="10">10</option>
<option value="11">11</option>
</select>
</div>
<div class="left">
<button id="downloadbutton" class="btn download" onclick="downloadTrackImages()">Download</button>
</div></div>
</div>

<script src="https://cdnjs.cloudflare.com/ajax/libs/FileSaver.js/2.0.0/FileSaver.min.js" integrity="sha512-csNcFYJniKjJxRWRV1R7fvnXrycHP6qDR21mgz1ZP55xY5d+aHLfo9/FcGDQLfn2IfngbAHd8LdfsagcCqgTcQ==" crossorigin="anonymous" referrerpolicy="no-referrer"></script>
<script src="https://cdnjs.cloudflare.com/ajax/libs/jszip/3.10.1/jszip.min.js" integrity="sha512-XMVd28F1oH/O71fzwBnV7HucLxVwtxf26XV8P4wPk26EDxuGZ91N8bsOttmnomcCD3CS5ZMRL50H0GgOHvegtg==" crossorigin="anonymous" referrerpolicy="no-referrer"></script>
<script>
// download multipart image
var duringDownload = false;
function downloadTrackImages() {{
    if (duringDownload) {{
        return;
    }}
    duringDownload = true;
    var downloadButton = document.getElementById("downloadbutton");
    // button name change
    downloadButton.innerText = "Downloading...";
    var random_number = Math.floor(Math.random()*10000);
    var trackid = document.getElementById("trackidSelect").value;
    var xhr = new XMLHttpRequest();
    xhr.open("GET", "/images?trackid=" + trackid + "&fromframe=0&toframe=-1&random_number=" + random_number, true);
    xhr.responseType = "blob";
    xhr.addEventListener("progress", function(event) {{
        // download status
        var progress = event.loaded;
        downloadButton.innerText = "Downloading " + progress + "Bytes";
    }});
    xhr.onload = function() {{
        var url = window.URL.createObjectURL(xhr.response);
        // get multipart data
        var reader = new FileReader();
        reader.onload = function() {{
            var data = reader.result;
            var parts = data.split("--timeleapcamboundary");
            var zip = new JSZip();
            for (var i = 1; i < parts.length; i++) {{
                var part = parts[i];
                var lines = part.split("\r\n");
                var content_type = lines[1].split(": ")[1];
                var filename = lines[2].split("=")[1];
                var images = part.split("\r\n\r\n");
                var image = atob(images[1]);
                zip.file(filename, image, {{binary: true}});
            }}
            zip.generateAsync({{type:"blob"}}).then(function(content) {{
                var savefile = "image_" + trackid + ".zip";
                saveAs(content, savefile);
                duringDownload = false;
                downloadButton.innerText = "Download";
            }});
        }};
        reader.readAsText(xhr.response);
    }};
    xhr.send();
}}

function drawImageOnWindow(trackid, fromframe, toframe) {{
    var random_number = Math.floor(Math.random()*10000);
    window.open('/data?trackid=' + trackid + '&fromframe=' + fromframe + '&toframe=' + toframe + '&random_number=' + random_number);
}}

function drawThumbnail(trackid, canvasid) {{
    var random_number = Math.floor(Math.random()*10000);
    var canvas = document.getElementById(canvasid);
    var ctx = canvas.getContext("2d");
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    var img = new Image();
    img.onload = function() {{
        ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
    }};
    img.src = "/data?trackid=" + trackid + "&fromframe=0&toframe=0&random_number=" + random_number;
}}

function drawAllThumbnail() {{
    for (var i = 0; i < 12; i++) {{
        drawThumbnail(i, "canvas" + i);
    }}
}}

drawAllThumbnail();

</script>
</body>
</html>
"#)
}

fn monitor_html() -> String {
    format!(
        r#"
<!DOCTYPE HTML><html>
<head>
    <title>Time Leap Cam</title>
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <style>
    html {{font-family: Times New Roman; display: inline-block; text-align: center;}}
    body {{max-width: 900px; margin:0px auto; padding-bottom: 25px;}}
    .switch {{position: relative; display: inline-block; width: 60px; height: 34px;}} 
    .switch input {{display: none}}
    .slider {{position: absolute; top: 0; left: 0; right: 0; bottom: 0; background-color: #FF0000; border-radius: 34px}}
    .slider:before {{position: absolute; content: ""; height: 26px; width: 26px; left: 4px; bottom: 4px; background-color: #fff; -webkit-transition: .4s; transition: .4s; border-radius: 34px}}
    input:checked+.slider {{background-color: #27c437}}
    input:checked+.slider:before {{-webkit-transform: translateX(26px); -ms-transform: translateX(26px); transform: translateX(26px)}}
    .topnav {{ background-color: #1206d7; overflow: hidden}}
    .topnav a {{ float: left; color: #f2f2f2; text-align: center; padding: 14px 16px; text-decoration: none; font-size: 17px}}
    .topnav a:hover {{ background-color: #ddd; color: black}}
    .topnav a.active {{ background-color: #0dc044; color: white}}
    .left {{ float: left; width: 50%; font-size: 1.5rem; text-align: left;}}
    .center {{ float: left; width: 50%; font-size: 1.5rem; text-align: center;}}
    .clear {{ clear: both;}}
    </style>
</head>

<body>
<div class="topnav">
  <a href="/">CAPTURE</a>
  <a href="image.html">IMAGE</a>
  <a class="active" href="monitor.html">MONITORING</a>
  <a href="config.html">CONFIG</a>
  <a href="status.html">STATUS</a>
</div>
<div style="padding:20px;">
<div class="left">
<label for="queryopenai">Monitoring:</label></div>
<div class="left">
<label class="switch"><input type="checkbox" onchange="toggleCheckbox(this)" id="queryopenai">
<span class="slider"></span></label>
</div>
<div class="clear">
<div class="left">
<label for="queryprompt">Prompt:</label></div>
</div>
<div class="leftall">
<textarea id="queryprompt" name="" rows="4" cols="50">Input Query Prompt</textarea>
</div>
</div>

<script>
function toggleCheckbox(element) {{
    var queryopenai = document.getElementById("queryopenai");
    if (element.checked) {{
        MonitoringStart();
    }} else {{
        MonitoringStop();
    }}
}}

function getConfig() {{
    var xhttp = new XMLHttpRequest();
    xhttp.onreadystatechange = function() {{
        if (this.readyState == 4 && this.status == 200) {{
            var config = JSON.parse(this.responseText);
            if (config.queryopenai == true) {{
                document.getElementById("queryopenai").checked = true;
            }} else {{
                document.getElementById("queryopenai").checked = false;
            }}
            document.getElementById("queryprompt").value = config.queryprompt;
        }}
    }};
    xhttp.open("GET", "/config", true);
    xhttp.send();
}};

getConfig();

function MonitoringStart() {{
    var queryopenai_element = document.getElementById("queryopenai");
    var queryprompt_element = document.getElementById("queryprompt");
    var queryopenai = queryopenai_element.checked;
    var queryprompt = queryprompt_element.value;
    var xhr = new XMLHttpRequest();
    xhr.open("POST", "/monitor", true);
    xhr.setRequestHeader("Content-Type", "application/json");
    xhr.send(JSON.stringify({{
        "queryopenai": queryopenai,
        "queryprompt": queryprompt
    }}));
}}
function MonitoringStop() {{
    var xhr = new XMLHttpRequest();
    xhr.open("POST", "/monitor", true);
    xhr.setRequestHeader("Content-Type", "application/json");
    xhr.send(JSON.stringify({{
        "queryopenai": false,
    }}));
}}
</script>
</body>
</html>
"#)
}

fn index_html(status: bool) -> String {
    format!(
        r#"
<!DOCTYPE HTML><html>
<head>
    <title>Time Leap Cam</title>
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <style>
    html {{font-family: Times New Roman; display: inline-block; text-align: center;}}
    body {{max-width: 900px; margin:0px auto; padding-bottom: 25px;}}
    .switch {{position: relative; display: inline-block; width: 60px; height: 34px}} 
    .switch input {{display: none}}
    .slider {{position: absolute; top: 0; left: 0; right: 0; bottom: 0; background-color: #FF0000; border-radius: 34px}}
    .slider:before {{position: absolute; content: ""; height: 26px; width: 26px; left: 4px; bottom: 4px; background-color: #fff; -webkit-transition: .4s; transition: .4s; border-radius: 34px}}
    input:checked+.slider {{background-color: #27c437}}
    input:checked+.slider:before {{-webkit-transform: translateX(26px); -ms-transform: translateX(26px); transform: translateX(26px)}}
    .thumbnail {{ cursor: pointer; width: 492px; height: 500px; margin: 0 auto}}
    .topnav {{ background-color: #1206d7; overflow: hidden}}
    .topnav a {{ float: left; color: #f2f2f2; text-align: center; padding: 14px 16px; text-decoration: none; font-size: 17px}}
    .topnav a:hover {{ background-color: #ddd; color: black}}
    .topnav a.active {{ background-color: #0dc044; color: white}}
    .left {{ float: left; width: 50%; font-size: 1.5rem; text-align: left;}}
    .center {{ float: left; width: 50%; font-size: 1.5rem; text-align: center;}}
    .right {{ float: left; width: 50%; text-align: left; font-size: 1.0rem;}}
    .clear {{ clear: both;}}
    </style>
</head>

<body>
<div class="topnav">
  <a class="active" href="/">CAPTURE</a>
  <a href="image.html">IMAGE</a>
  <a href="monitor.html">MONITORING</a>
  <a href="config.html">CONFIG</a>
  <a href="status.html">STATUS</a>
</div>
<div style="padding:20px;">
<div class="left">
<label for="captureStart">Capture Start:</label></div>
<div class="left">
<label class="switch"><input type="checkbox" onchange="toggleCheckbox(this)" id="captureStart" {} >
<span class="slider"></span></label>
</div>
<div class="clear">
<div class="left">
<label for="resolutionSelect">Resolution:</label></div>
<div class="left">
<select id="resolutionSelect" onchange="setResolution(this)">
<option value="QVGA">QVGA 320x240</option>
<option value="CIF">CIF 400x296</option>
<option value="HVGA">HVGA 480x320</option>
<option value="VGA">VGA 640x480</option>
<option value="SVGA" selected>SVGA 800x600</option>
<option value="XGA">XGA 1024x768</option>
<option value="HD">HD 1280x720</option>
<option value="SXGA">SXGA 1280x1024</option>
<option value="UXGA">UXGA 1600x1200</option>
<option value="FHD">FHD 1920x1080</option>
<option value="QXGA">QXGA 2048x1536</option>
<option value="QSXGA">QSXGA 2592x1944</option>
<option value="WQXGA">WQXGA 2560x1600</option>
<option value="QHD">QHD 2560x1440</option>
</select>
</div>
<div class="clear">
<div class="left">
<label for="trackidSelect">Track ID:</label></div>
<div class="left">
<select id="trackidSelect" onchange="drawPreview()">
<option value="0">0</option>
<option value="1">1</option>
<option value="2">2</option>
<option value="3">3</option>
<option value="4">4</option>
<option value="5">5</option>
<option value="6">6</option>
<option value="7">7</option>
<option value="8">8</option>
<option value="9">9</option>
<option value="10">10</option>
<option value="11">11</option>
</select>
</div></div>

<div class="clear">
<div class="left">
<label for="durationSelect">Duration:</label></div>
<div class="left">
<select id="durationSelect">
<option value="0">None</option>
<option value="10">10</option>
<option value="20">20</option>
<option value="30">30</option>
<option value="40">40</option>
<option value="50">50</option>
<option value="60">60</option>
<option value="90">90</option>
<option value="120">2min</option>
<option value="150">2.5min</option>
<option value="180">3min</option>
<option value="300">5min</option>
<option value="600">10min</option>
<option value="900">15min</option>
<option value="1200">20min</option>
<option value="1800">30min</option>
<option value="3600">1hr</option>
<option value="7200">2hr</option>
<option value="10800">3hr</option>
<option value="14400">4hr</option>
<option value="18000">5hr</option>
<option value="21600">6hr</option>
<option value="25200">7hr</option>
<option value="28800">8hr</option>
<option value="32400">9hr</option>
<option value="36000">10hr</option>
<option value="39600">11hr</option>
<option value="43200">12hr</option>
</select>
</div></div>

<div class="clear">
<div class="left">
<label for="leapday">Leap Date Time:</label></div>
<div class="right">*-*-
<select id="leapday">
<option value="-1">*</option>
<option value="01">01</option>
<option value="02">02</option>
<option value="03">03</option>
<option value="04">04</option>
<option value="05">05</option>
<option value="06">06</option>
<option value="07">07</option>
<option value="08">08</option>
<option value="09">09</option>
<option value="10">10</option>
<option value="11">11</option>
<option value="12">12</option>
<option value="13">13</option>
<option value="14">14</option>
<option value="15">15</option>
<option value="16">16</option>
<option value="17">17</option>
<option value="18">18</option>
<option value="19">19</option>
<option value="20">20</option>
<option value="21">21</option>
<option value="22">22</option>
<option value="23">23</option>
<option value="24">24</option>
<option value="25">25</option>
<option value="26">26</option>
<option value="27">27</option>
<option value="28">28</option>
<option value="29">29</option>
<option value="30">30</option>
<option value="31">31</option>
</select>  <select id="leaphour">
<option value="-1">*</option>
<option value="00">00</option>
<option value="01">01</option>
<option value="02">02</option>
<option value="03">03</option>
<option value="04">04</option>
<option value="05">05</option>
<option value="06">06</option>
<option value="07">07</option>
<option value="08">08</option>
<option value="09">09</option>
<option value="10">10</option>
<option value="11">11</option>
<option value="12">12</option>
<option value="13">13</option>
<option value="14">14</option>
<option value="15">15</option>
<option value="16">16</option>
<option value="17">17</option>
<option value="18">18</option>
<option value="19">19</option>
<option value="20">20</option>
<option value="21">21</option>
<option value="22">22</option>
<option value="23">23</option>
</select>:<select id="leapminute">
<option value="-1">*</option>
<option value="00">00</option>
<option value="01">01</option>
<option value="02">02</option>
<option value="03">03</option>
<option value="04">04</option>
<option value="05">05</option>
<option value="06">06</option>
<option value="07">07</option>
<option value="08">08</option>
<option value="09">09</option>
<option value="10">10</option>
<option value="11">11</option>
<option value="12">12</option>
<option value="13">13</option>
<option value="14">14</option>
<option value="15">15</option>
<option value="16">16</option>
<option value="17">17</option>
<option value="18">18</option>
<option value="19">19</option>
<option value="20">20</option>
<option value="21">21</option>
<option value="22">22</option>
<option value="23">23</option>
<option value="24">24</option>
<option value="25">25</option>
<option value="26">26</option>
<option value="27">27</option>
<option value="28">28</option>
<option value="29">29</option>
<option value="30">30</option>
<option value="31">31</option>
<option value="32">32</option>
<option value="33">33</option>
<option value="34">34</option>
<option value="35">35</option>
<option value="36">36</option>
<option value="37">37</option>
<option value="38">38</option>
<option value="39">39</option>
<option value="40">40</option>
<option value="41">41</option>
<option value="42">42</option>
<option value="43">43</option>
<option value="44">44</option>
<option value="45">45</option>
<option value="46">46</option>
<option value="47">47</option>
<option value="48">48</option>
<option value="49">49</option>
<option value="50">50</option>
<option value="51">51</option>
<option value="52">52</option>
<option value="53">53</option>
<option value="54">54</option>
<option value="55">55</option>
<option value="56">56</option>
<option value="57">57</option>
<option value="58">58</option>
<option value="59">59</option>
</select>:0</div></div>
<div class="clear">
<div class="left">
<label for="captureStartTime">Capture Start Time:</label></div>
<div class="left">
<input type="datetime-local" id="captureStartTime" name="captureStartTime" value="2021-01-01T00:00:00"></div></div>
<div class="clear">
<div class="left">
<label for="captureEndTime">Capture End Time:</label></div>
<div class="left">
<input type="datetime-local" id="captureEndTime" name="captureEndTime" value="2021-01-01T00:00:00"></div></div>
<div class="left">
<label for="captureFramesAtOnce">Capture Frames At Once (sec):</label></div>
<div class="left">
<input type="number" id="captureFramesAtOnce" value="1"></div></div>
<div class="clear">
<div class="left">
<label for="OverwriteSaved">Over Write Save:</label></div>
<div class="left">
<label class="switch"><input type="checkbox" id="OverwriteSaved">
<span class="slider"></span></label>
</div></div>
<div class="clear">
<canvas id="preview" width="320" height="240" onclick="getOneShot()"></canvas>
</div>
</div>


<script>
function toggleCheckbox(element) {{
    if(element.checked){{
        CaptureStart();
    }}
    else {{
        CaptureStop();
        drawAllThumbnail();
    }}
}}

function setResolution(element) {{
    var xhr = new XMLHttpRequest();
    xhr.open("POST", "/resolution", true);
    xhr.setRequestHeader("Content-Type", "application/json");
    xhr.send(JSON.stringify({{
        "resolution": element.value
    }}));
}}

function CaptureStart() {{
    var resolution_element = document.getElementById("resolutionSelect");
    var trackid_element = document.getElementById("trackidSelect");
    var duration_element = document.getElementById("durationSelect");
    var leapday_element = document.getElementById("leapday");
    var leaphour_element = document.getElementById("leaphour");
    var leapminute_element = document.getElementById("leapminute");
    var xhr = new XMLHttpRequest();
    xhr.open("POST", "/capture", true);
    xhr.setRequestHeader("Content-Type", "application/json");
    xhr.send(JSON.stringify({{
        "request": "start",
        "resolution": resolution_element.value,
        "trackid":  trackid_element.value - 0,
        "duration": duration_element.value - 0,
        "leaptime": {{
            "day": leapday_element.value - 0,
            "hour": leaphour_element.value - 0,
            "minute": leapminute_element.value - 0,
            "second": -1
        }},
        "captureStartTime": document.getElementById("captureStartTime").value,
        "captureEndTime": document.getElementById("captureEndTime").value,
        "captureFramesAtOnce": document.getElementById("captureFramesAtOnce").value - 0,
        "overwriteSaved": document.getElementById("OverwriteSaved").checked,
    }}));
}}
function CaptureStop() {{
    var xhr = new XMLHttpRequest();
    xhr.open("POST", "/capture", true);
    xhr.setRequestHeader("Content-Type", "application/json");
    xhr.send(JSON.stringify({{
        "request": "stop",
        "trackid": 1,
        "duration": 0,
        "leaptime": {{
            "day": 0,
            "hour": 0,
            "minute": 0,
            "second": 0
        }}
    }}));
}}

// get current configuration from /config by GET method
function getConfig() {{
    var xhttp = new XMLHttpRequest();
    xhttp.onreadystatechange = function() {{
        if (this.readyState == 4 && this.status == 200) {{
            var config = JSON.parse(this.responseText);
            document.getElementById("resolutionSelect").value = config.resolution;
            document.getElementById("trackidSelect").value = config.trackid;
            document.getElementById("durationSelect").value = config.duration;
            document.getElementById("captureStartTime").value = config.captureStartTime;
            document.getElementById("captureEndTime").value = config.captureEndTime;
            document.getElementById("leapday").value = config.leaptime.day;
            document.getElementById("leaphour").value = config.leaptime.hour;
            document.getElementById("leapminute").value = config.leaptime.minute;
            document.getElementById("captureFramesAtOnce").value = config.captureFramesAtOnce;
            document.getElementById("jpegQuality").value = config.jpegQuality;
            document.getElementById("OverwriteSaved").checked = config.overwriteSaved;
            document.getElementById("directWriteMode").checked = config.directWriteMode;
        }}
    }};
    xhttp.open("GET", "/config", true);
    xhttp.send();
}};

function drawPreview() {{
    var random_number = Math.floor(Math.random()*10000);
    var trackid = document.getElementById("trackidSelect").value;
    var canvas = document.getElementById("preview");
    var ctx = canvas.getContext("2d");
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    var img = new Image();
    img.onload = function() {{
        ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
    }};
    img.src = "/data?trackid=" + trackid + "&fromframe=-1&toframe=-1&random_number=" + random_number;
}}

var check_completed = null;

function getOneShot() {{
    if (check_completed != null) {{
        return;
    }}
    if (captureStart.checked == true) {{
        return;
    }}
    var random_number = Math.floor(Math.random()*10000);
    var trackid = document.getElementById("trackidSelect").value;
    var xhr = new XMLHttpRequest();
    var canvas = document.getElementById("preview");
    var ctx = canvas.getContext("2d");
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    xhr.open("POST", "/oneshot", true);
    xhr.setRequestHeader("Content-Type", "application/json");
    xhr.send(JSON.stringify({{
        "trackid": trackid - 0,
    }}));

    // draw capturing text message
    ctx.font = "30px Arial";
    ctx.fillText("Capturing Now...", 10, 50);

    check_completed = setInterval(function ( ) {{
        var xhr = new XMLHttpRequest();
        xhr.onreadystatechange = function() {{
            if (this.readyState == 4 && this.status == 200) {{
                var completion = JSON.parse(this.responseText);
                if (completion.status == true) {{
                    drawPreview();
                    clearInterval(check_completed);
                    check_completed = null;
                }}
            }}
        }};
        xhr.open("GET", "/oneshot", true);
        xhr.send();
    }}, 1000 ) ;
}}

getConfig();
drawPreview();

</script>
</body>
</html>
"#, if status { "checked" } else { "" }
    )
}

fn status_html() -> String {
    format!(
        r#"
<!DOCTYPE HTML><html>
<head>
    <title>Time Leap Cam</title>
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <style>
    html {{font-family: Times New Roman; display: inline-block; text-align: center;}}
    h2 {{font-size: 3.0rem;}}
    h4 {{font-size: 2.0rem;}}
    h5 {{font-size: 1.0rem; text-align: left;}}
    p {{font-size: 3.0rem;}}
    body {{max-width: 900px; margin:0px auto; padding-bottom: 25px;}}
    .switch {{position: relative; display: inline-block; width: 120px; height: 68px}} 
    .switch input {{display: none}}
    .slider {{position: absolute; top: 0; left: 0; right: 0; bottom: 0; background-color: #FF0000; border-radius: 34px}}
    .slider:before {{position: absolute; content: ""; height: 52px; width: 52px; left: 8px; bottom: 8px; background-color: #fff; -webkit-transition: .4s; transition: .4s; border-radius: 68px}}
    input:checked+.slider {{background-color: #27c437}}
    input:checked+.slider:before {{-webkit-transform: translateX(52px); -ms-transform: translateX(52px); transform: translateX(52px)}}
    .thumbnail {{ cursor: pointer; width: 492px; height: 500px; margin: 0 auto}}
    .topnav {{ background-color: #1206d7; overflow: hidden}}
    .topnav a {{ float: left; color: #f2f2f2; text-align: center; padding: 14px 16px; text-decoration: none; font-size: 17px}}
    .topnav a:hover {{ background-color: #ddd; color: black}}
    .topnav a.active {{ background-color: #0dc044; color: white}}
    .left {{ float: left; width: 50%; font-size: 1.5rem; text-align: left;}}
    .right {{ float: right; width: 50%; text-align: center; font-size: 2.0rem;}}
    .clear {{ clear: both;}}
    </style>
</head>

<body>
<div class="topnav">
  <a href="/">CAPTURE</a>
  <a href="image.html">IMAGE</a>
  <a href="monitor.html">MONITORING</a>
  <a href="config.html">CONFIG</a>
  <a class="active" href="status.html">STATUS</a>
</div>
<div style="padding:20px;">
<div class="left">
<label for="camState">Connection:</label></div>
<div class="left"><span id="camState"><span></div>

<div class="clear">
<div class="left">
<label for="batteryVoltage">Battery Voltage:</label></div>
<div class="left"><span id="batteryVoltage"><span>V</div>
</div>

<div class="clear">
<div class="left">
<label for="wifiRSSI">WiFi RSSI:</label></div>
<div class="left"><span id="wifiRSSI"><span></div>
</div>

<div class="clear">
<div class="left">
<label for="captureID">Capture ID:</label></div>
<div class="left"><span id="captureID"><span></div>
</div>

<div class="clear">
<div class="left">
<label for="lastCaptureDateTime">Last Capture:</label></div>
<div class="left"><span id="lastCaptureDateTime"><span></div>
</div>

<div class="clear">
<div class="left">
<label for="lastpostedDateTime">Posted Time:</label></div>
<div class="left"><span id="lastpostedDateTime"><span></div>
</div>

<div class="clear">
<div class="left">
<label for="temperature">Temp.</label></div>
<div class="left"><span id="temperature"><span></div>
</div></div>

<script>

function get_state () {{
    var xhttp = new XMLHttpRequest();
    xhttp.onreadystatechange = function() {{
        if (this.readyState == 4 && this.status == 200) {{
            var status = JSON.parse(this.responseText);
            var camState = "Connected";
            document.getElementById("camState").innerHTML = camState;
            var batteryVoltage = status.battery_voltage;
            var wifiRSSI = status.rssi;
            document.getElementById("batteryVoltage").innerHTML = batteryVoltage+"V";
            document.getElementById("wifiRSSI").innerHTML = wifiRSSI+"dBm";
            document.getElementById("captureID").innerHTML = status.capture_id;
            document.getElementById("lastCaptureDateTime").innerHTML = status.last_capture_date_time;
            document.getElementById("lastpostedDateTime").innerHTML = status.last_posted_date_time;
            document.getElementById("temperature").innerHTML = status.temperature+"C";
        }}
        else if (this.readyState == 4 && this.status == 0) {{
            document.getElementById("camState").innerHTML = "Not Connected";
        }}
    }};
    xhttp.open("GET", "/state", true);
    xhttp.send();
}}

setInterval(function ( ) {{
    get_state();
}}, 10000 ) ;

get_state();
</script>
</body>
</html>
"#)
}

fn config_html() -> String {
    format!(
        r#"
<!DOCTYPE HTML><html>
<head>
    <title>Time Leap Cam</title>
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <style>
    html {{font-family: Times New Roman; display: inline-block; text-align: center;}}
    body {{max-width: 900px; margin:0px auto; padding-bottom: 25px;}}
    .topnav {{ background-color: #1206d7; overflow: hidden}}
    .topnav a {{ float: left; color: #f2f2f2; text-align: center; padding: 14px 16px; text-decoration: none; font-size: 17px}}
    .topnav a:hover {{ background-color: #ddd; color: black}}
    .topnav a.active {{ background-color: #0dc044; color: white}}
    .left {{ float: left; width: 50%; font-size: 1.5rem; text-align: left;}}
    .center {{ float: left; width: 100%; font-size: 1.5rem; text-align: center;}}
    .clear {{ clear: both;}}
    .btn {{ border: 2px solid black; border-radius: 5px; background-color: white; color: black; padding: 10px 28px; font-size: 16px; cursor: pointer; margin: 8px 4px;}}
    .save {{ border-color: #04AA6D; color: green; }}
    .save:hover {{ background-color: #04AA6D; color: white; }}
    </style>
</head>

<body>
<div class="topnav">
  <a href="/">CAPTURE</a>
  <a href="image.html">IMAGE</a>
  <a href="monitor.html">MONITORING</a>
  <a class="active" href="config.html">CONFIG</a>
  <a href="status.html">STATUS</a>
</div>
<div style="padding:20px;">
<div class="left">
<label for="resolutionSelect">Default Resolution:</label></div>
<div class="left">
<select id="resolutionSelect" onchange="setResolution(this)">
<option value="QVGA">QVGA 320x240</option>
<option value="CIF">CIF 400x296</option>
<option value="HVGA">HVGA 480x320</option>
<option value="VGA">VGA 640x480</option>
<option value="SVGA" selected>SVGA 800x600</option>
<option value="XGA">XGA 1024x768</option>
<option value="HD">HD 1280x720</option>
<option value="SXGA">SXGA 1280x1024</option>
<option value="UXGA">UXGA 1600x1200</option>
<option value="FHD">FHD 1920x1080</option>
<option value="QXGA">QXGA 2048x1536</option>
<option value="QSXGA">QSXGA 2592x1944</option>
<option value="WQXGA">WQXGA 2560x1600</option>
<option value="QHD">QHD 2560x1440</option>
</select>
</div>
<div class="clear">
<div class="left">
<label for="trackidSelect">Track ID:</label></div>
<div class="left">
<select id="trackidSelect">
<option value="0">0</option>
<option value="1">1</option>
<option value="2">2</option>
<option value="3">3</option>
<option value="4">4</option>
<option value="5">5</option>
<option value="6">6</option>
<option value="7">7</option>
<option value="8">8</option>
<option value="9">9</option>
<option value="10">10</option>
<option value="11">11</option>
</select>
</div></div>

<div class="clear">
<div class="left">
<label for="durationSelect">Duration:</label></div>
<div class="left">
<select id="durationSelect">
<option value="0">None</option>
<option value="10">10</option>
<option value="20">20</option>
<option value="30">30</option>
<option value="40">40</option>
<option value="50">50</option>
<option value="60">60</option>
<option value="90">90</option>
<option value="120">2min</option>
<option value="150">2.5min</option>
<option value="180">3min</option>
<option value="300">5min</option>
<option value="600">10min</option>
<option value="900">15min</option>
<option value="1200">20min</option>
<option value="1800">30min</option>
<option value="3600">1hr</option>
<option value="7200">2hr</option>
<option value="10800">3hr</option>
<option value="14400">4hr</option>
<option value="18000">5hr</option>
<option value="21600">6hr</option>
<option value="25200">7hr</option>
<option value="28800">8hr</option>
<option value="32400">9hr</option>
<option value="36000">10hr</option>
<option value="39600">11hr</option>
<option value="43200">12hr</option>
</select>
</div></div>

<div class="clear">
<div class="left">
<label for="timezoneSelect">Time Zone:</label></div>
<div class="left">
<select id="timezoneSelect">
<option value="-12">UTC-12</option>
<option value="-11">UTC-11</option>
<option value="-10">UTC-10</option>
<option value="-9">UTC-9</option>
<option value="-8">UTC-8</option>
<option value="-7">UTC-7</option>
<option value="-6">UTC-6</option>
<option value="-5">UTC-5</option>
<option value="-4">UTC-4</option>
<option value="-3">UTC-3</option>
<option value="-2">UTC-2</option>
<option value="-1">UTC-1</option>
<option value="0">UTC</option>
<option value="1">UTC+1</option>
<option value="2">UTC+2</option>
<option value="3">UTC+3</option>
<option value="4">UTC+4</option>
<option value="5">UTC+5</option>
<option value="6">UTC+6</option>
<option value="7">UTC+7</option>
<option value="8">UTC+8</option>
<option value="9">UTC+9</option>
<option value="10">UTC+10</option>
<option value="11">UTC+11</option>
<option value="12">UTC+12</option>
</select></div></div>

<div class="clear">
<div class="left">
<label for="idlesleep">Sleep Time:</label></div>
<div class="left">
<input type="number" id="idlesleep" value="300">
</div></div>
<div class="clear">
<div class="left">
<label for="autocaptureSelect">Auto Capture:</label></div>
<div class="left">
<select id="autocaptureSelect">
<option value="true">True</option>
<option value="false">False</option>
</select>
</div></div>
<div class="clear">
<div class="left">
<label for="openaiSelect">OpenAI Model:</label></div>
<div class="left">
<select id="openaiSelect">
<option value="gpt-4o-mini">GPT-4o mini</option>
<option value="gpt-4o">GPT-4o</option>
<option value="gpt-4-turbo">GPT-4-Turbo</option>
</select>
</div></div>
<div class="clear">
<div class="left">
<label for="autofocusOnce">Auto Focus Timing:</label></div>
<div class="left">
<select id="autofocusOnce">
<option value="true">Auto Focus Once</option>
<option value="false">Everytime Auto Focus</option>
</select>
</div></div>
<div class="clear">
<div class="left">
<label for="status_report">Status Report:</label></div>
<div class="left">
<select id="status_report">
<option value="true">True</option>
<option value="false">False</option>
</select>
</div></div>

<div class="clear">
<div class="left">
<label for="status_report">Report Interval:</label></div>
<div class="left">
<input type="number" id="status_report_interval" value="300">
</div></div>

<div class="clear">
<div class="left">
<label for="post_interval">Post Interval(sec):</label></div>
<div class="left">
<input type="number" id="post_interval" value="300">
</div></div>

<div class="clear">
<div class="left">
<label for="captureFramesAtOnce">Capture Frames At Once (sec):</label></div>
<div class="left">
<input type="number" id="captureFramesAtOnce" value="1">
</div></div>

<div class="clear">
<div class="left">
<label for="jpegQuality">JPEG Quality 4(High)-40(Low):</label></div>
<div class="left">
<input type="number" id="jpegQuality" value="12">
</div></div>

<div class="clear">
<div class="left">
<label for="OverwriteSaved">Over Write Save:</label></div>
<div class="left">
<label class="switch"><input type="checkbox" id="OverwriteSaved">
<span class="slider"></span></label>
</div></div>

<div class="clear">
<div class="left">
<label for="directWriteMode">Direct Write Mode:</label></div>
<div class="left">
<label class="switch"><input type="checkbox" id="directWriteMode">
<span class="slider"></span></label>
</div></div>

<div class="clear"> </div>
<div class="center">
<button class="btn save" onclick="saveConfig()">Save</button>
</div>
</div>

<script>
function saveConfig() {{
    var resolution_element = document.getElementById("resolutionSelect");
    var trackid_element = document.getElementById("trackidSelect");
    var duration_element = document.getElementById("durationSelect");
    var timezone_element = document.getElementById("timezoneSelect");
    var idlesleep_element = document.getElementById("idlesleep");
    var autocapture_element = document.getElementById("autocaptureSelect");
    var openai_element = document.getElementById("openaiSelect");
    var status_report_element = document.getElementById("status_report");
    var status_report_interval_element = document.getElementById("status_report_interval");
    var post_interval_element = document.getElementById("post_interval");
    var captureFramesAtOnce_element = document.getElementById("captureFramesAtOnce");
    var jpegQuality_element = document.getElementById("jpegQuality");
    var overwriteSaved_element = document.getElementById("OverwriteSaved");
    var directWriteMode_element = document.getElementById("directWriteMode");
    var xhr = new XMLHttpRequest();
    xhr.open("POST", "/config", true);
    xhr.setRequestHeader("Content-Type", "application/json");
    xhr.send(JSON.stringify({{
        "resolution": resolution_element.value,
        "trackid": trackid_element.value - 0,
        "duration": duration_element.value - 0,
        "timezone": timezone_element.value - 0,
        "idlesleep": idlesleep_element.value - 0,
        "autocapture": autocapture_element.value,
        "openai_model": openai_element.value,
        "autofocus_once": document.getElementById("autofocusOnce").value,
        "status_report": status_report_element.value,
        "status_report_interval": status_report_interval_element.value - 0,
        "post_interval": post_interval_element.value - 0,
        "captureFramesAtOnce": captureFramesAtOnce_element.value - 0,
        "jpegQuality": jpegQuality_element.value - 0,
        "overwriteSaved": overwriteSaved_element.checked,
        "directWriteMode": directWriteMode_element.checked,
    }}));
}}

// get current configuration from /config by GET method
function getConfig() {{
    var xhttp = new XMLHttpRequest();
    xhttp.onreadystatechange = function() {{
        if (this.readyState == 4 && this.status == 200) {{
            var config = JSON.parse(this.responseText);
            document.getElementById("resolutionSelect").value = config.resolution;
            document.getElementById("trackidSelect").value = config.trackid;
            document.getElementById("durationSelect").value = config.duration;
            document.getElementById("timezoneSelect").value = config.timezone;
            document.getElementById("idlesleep").value = config.idlesleep;
            document.getElementById("autocaptureSelect").value = config.autocapture;
            document.getElementById("openaiSelect").value = config.openai_model;
            document.getElementById("autofocusOnce").value = config.autofocus_once;
            document.getElementById("status_report").value = config.status_report;
            document.getElementById("status_report_interval").value = config.status_report_interval;
            document.getElementById("post_interval").value = config.post_interval;
            document.getElementById("captureFramesAtOnce").value = config.captureFramesAtOnce;
            document.getElementById("jpegQuality").value = config.jpegQuality;
            document.getElementById("OverwriteSaved").checked = config.overwriteSaved;
            document.getElementById("directWriteMode").checked = config.directWriteMode;
        }}
    }};
    xhttp.open("GET", "/config", true);
    xhttp.send();
}};

getConfig();
</script>
</body>
</html>
"#)
}
