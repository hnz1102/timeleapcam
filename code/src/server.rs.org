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

use crate::imagefiles;
//use crate::capture::Capture;

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
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
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
    pub latest_access_time: SystemTime,
    pub query_openai: bool,
    pub query_prompt: String,
    pub openai_model: String,
    pub rssi: i32,
    pub battery_voltage: f32,
    pub current_capture_id: u32,
    pub last_capture_date_time: SystemTime,
}

impl ControlServerInfo {
    pub fn new() -> ControlServerInfo {
        ControlServerInfo {
            need_to_save: false,
            capture_started: false,
            track_id: 0,
            duration: 90,
            leap_time: LeapTime {
                year: 2024,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
            },
            timezone: 9,
            resolution: camera::framesize_t_FRAMESIZE_VGA,
            idle_in_sleep_time: 300,
            auto_capture: false,
            latest_access_time: SystemTime::now(),
            query_openai: false,
            query_prompt: String::from(""),
            openai_model: String::from(""),
            rssi: 0,
            battery_voltage: 0.0,
            current_capture_id: 0,
            last_capture_date_time: SystemTime::now(),
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
                    let year = match leap_time.get("year") {
                        Some(year) => year.as_i64().unwrap(),
                        None => 0,
                    };
                    let month = match leap_time.get("month") {
                        Some(month) => month.as_u64().unwrap(),
                        None => 1,
                    };
                    let day = match leap_time.get("day") {
                        Some(day) => day.as_u64().unwrap(),
                        None => 1,
                    };
                    let hour = match leap_time.get("hour") {
                        Some(hour) => hour.as_u64().unwrap(),
                        None => 0,
                    };
                    let minute = match leap_time.get("minute") {
                        Some(minute) => minute.as_u64().unwrap(),
                        None => 0,
                    };
                    let second = match leap_time.get("second") {
                        Some(second) => second.as_u64().unwrap(),
                        None => 0,
                    };
                    LeapTime {
                        year: year as u32,
                        month: month as u32,
                        day: day as u32,
                        hour: hour as u32,
                        minute: minute as u32,
                        second: second as u32,
                    }
                }
                None => {
                    LeapTime {
                        year: 2024,
                        month: 1,
                        day: 1,
                        hour: 0,
                        minute: 0,
                        second: 0,
                    }
                }
            };
            info!("Leap Time: {:?}", leap_time);
            server_info.leap_time = leap_time;
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
            let response = request.into_ok_response();
            let status = if server_info.capture_started {
                "Capture started"
            } else {
                "Capture stopped"
            };
            server_info.latest_access_time = SystemTime::now();
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
                    server_info.latest_access_time = SystemTime::now();
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
            let mut response = request.into_response(200, Some("OK"), &headers).unwrap();
            let mut count = fromframe;
            let server_info_clone = server_info_get_image.clone();
            loop {
                if toframe >= 0 && count > toframe {
                    break;
                }
                let file_path = format!("/eMMC/T{}/I{}.jpg", trackid().unwrap(), count);
                let buffer = imagefiles::read_file(Path::new(&file_path));
                if buffer.len() == 0 {
                    break;
                }
                let mut server_info = server_info_clone.lock().unwrap();
                server_info.latest_access_time = SystemTime::now();
                drop(server_info);
                count += 1;
                response.write_all("--timeleapcamboundary\r\n".as_bytes())?;
                response.write_all("Content-Type: image/jpeg\r\n".as_bytes())?;
                let context_length = format!("Content-Length: {}\r\n\r\n", buffer.len());
                response.write_all(context_length.as_bytes())?;
                response.write_all(&buffer)?;
                response.write_all("\r\n".as_bytes())?;
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

        // button state by GET method
        let server_info_status = self.server_info.clone();
        self.http_server.fn_handler("/state", Method::Get, move |request| {
            let response = request.into_ok_response();
            let server_info = server_info_status.clone();
            let server_info = server_info.lock().unwrap();
            // state is capture_started status and rssi, battery_voltage values send as json format
            let fixed_offset = FixedOffset::east_opt(server_info.timezone * 3600).unwrap();
            let last_capture_date_time_utc: DateTime<Local> = server_info.last_capture_date_time.into();
            // adust timezone
            let last_capture_date_time = DateTime::<Local>::from_naive_utc_and_offset(last_capture_date_time_utc.naive_utc(), fixed_offset);
            let lcdt_str = last_capture_date_time.format("%Y-%m-%d %H:%M:%S").to_string();
            let state_json = format!("{{\"state\": \"{}\", \"rssi\": {}, \"battery_voltage\": {:.2}, \"capture_id\": {}, \"last_capture_date_time\": \"{}\"}}",
                                     if server_info.capture_started {
                                         "start"
                                     } else {
                                         "stop"
                                     },
                                     server_info.rssi,
                                     server_info.battery_voltage,
                                     server_info.current_capture_id,
                                     if server_info.last_capture_date_time == SystemTime::UNIX_EPOCH { "N/A" } else { &lcdt_str },
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
            // get query_openai
            let query_openai = match json["queryopenai"].as_str() {
                Some(query_openai) => {
                    if query_openai == "true" {
                        true
                    } else {
                        false
                    }  
                }
                None => {
                    false
                }
            };
            server_info.query_openai = query_openai;
            // get query_prompt
            let query_prompt = match json["queryprompt"].as_str() {
                Some(query_prompt) => query_prompt,
                None => {
                    ""
                }
            };
            // remove carriage return and line feed
            server_info.query_prompt = query_prompt.replace("\r", "").replace("\n", "").to_string();
            // get openai model
            let openai_model = match json["openai_model"].as_str() {
                Some(openai_model) => openai_model,
                None => {
                    ""
                }
            };
            server_info.openai_model = openai_model.to_string();
            server_info.need_to_save = true;
            server_info.latest_access_time = SystemTime::now();
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
            let config_json = format!("{{\"resolution\": \"{}\", \"trackid\": {}, \"duration\": {}, \"timezone\": {}, \"idlesleep\": {}, \"autocapture\": {}, \"queryopenai\": {}, \"queryprompt\": \"{}\", \"openai_model\": \"{}\"}}",
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
                                      server_info.openai_model);
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

    pub fn set_current_battery_voltage(&self, battery_voltage: f32) {
        let mut server_info = self.server_info.lock().unwrap();
        server_info.battery_voltage = battery_voltage;
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
    .thumbnail {{ cursor: pointer; width: 492px; height: 500px; margin: 0 auto; text-align: left;}}
    .topnav {{ background-color: #1206d7; overflow: hidden}}
    .topnav a {{ float: left; color: #f2f2f2; text-align: center; padding: 14px 16px; text-decoration: none; font-size: 17px}}
    .topnav a:hover {{ background-color: #ddd; color: black}}
    .topnav a.active {{ background-color: #0dc044; color: white}}
    .left {{ float: left; width: 50%; font-size: 2.0rem; text-align: left;}}
    .left33 {{ float: left; width: 33%; font-size: 2.0rem; text-align: left;}}
    .left3 {{ float: left; width: 50%; font-size: 3.0rem; text-align: center;}}
    .right {{ float: right; width: 50%; text-align: center; font-size: 2.0rem;}}
    .clear {{ clear: both;}}
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
</div></div></div>
<script>
function drawImageOnWindow(trackid, fromframe, toframe) {{
    var random_number = Math.floor(Math.random()*10000);
    window.open('/data?trackid=' + trackid + '&fromframe=' + fromframe + '&toframe=' + toframe + '&random_number=' + random_number);
}}

function drawThumbnail(trackid, canvasid) {{
    var random_number = Math.floor(Math.random()*10000);
    var canvas = document.getElementById(canvasid);
    var ctx = canvas.getContext("2d");
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    // draw image size to canvas size
    var img = new Image();
    img.onload = function() {{
        ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
    }};
    img.src = "/data?trackid=" + trackid + "&fromframe=0&toframe=0&random_number=" + random_number;
}}

function drawAllThumbnail() {{
    for (var i = 0; i < 11; i++) {{
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
    h2 {{font-size: 3.0rem;}}
    h4 {{font-size: 2.0rem;}}
    h5 {{font-size: 1.0rem; text-align: left;}}
    p {{font-size: 3.0rem;}}
    body {{max-width: 900px; margin:0px auto; padding-bottom: 25px;}}
    .switch {{position: relative; display: inline-block; width: 60px; height: 34px}} 
    .switch input {{display: none}}
    .slider {{position: absolute; top: 0; left: 0; right: 0; bottom: 0; background-color: #FF0000; border-radius: 34px}}
    .slider:before {{position: absolute; content: ""; height: 26px; width: 26px; left: 4px; bottom: 4px; background-color: #fff; -webkit-transition: .4s; transition: .4s; border-radius: 34px}}
    input:checked+.slider {{background-color: #27c437}}
    input:checked+.slider:before {{-webkit-transform: translateX(26px); -ms-transform: translateX(26px); transform: translateX(26px)}}
    .thumbnail {{ cursor: pointer; width: 492px; height: 500px; margin: 0 auto; text-align: left;}}
    .topnav {{ background-color: #1206d7; overflow: hidden}}
    .topnav a {{ float: left; color: #f2f2f2; text-align: center; padding: 14px 16px; text-decoration: none; font-size: 17px}}
    .topnav a:hover {{ background-color: #ddd; color: black}}
    .topnav a.active {{ background-color: #0dc044; color: white}}
    .left {{ float: left; width: 50%; font-size: 2.0rem; text-align: left;}}
    .leftall {{  clear: both; width: 100%; font-size: 2.0rem; text-align: left; }}
    .left33 {{ float: left; width: 33%; font-size: 2.0rem; text-align: left;}}
    .left3 {{ float: left; width: 50%; font-size: 3.0rem; text-align: center;}}
    .right {{ float: right; width: 50%; text-align: center; font-size: 2.0rem;}}
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
<label for="queryopenai">Monitoring Mode:</label></div>
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
        queryopenai.selectedIndex = 0;
    }} else {{
        queryopenai.selectedIndex = 1;
    }}
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
    h2 {{font-size: 3.0rem;}}
    h4 {{font-size: 2.0rem;}}
    h5 {{font-size: 1.0rem; text-align: left;}}
    p {{font-size: 3.0rem;}}
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
    .left {{ float: left; width: 50%; font-size: 2.0rem; text-align: left;}}
    .left3 {{ float: left; width: 50%; font-size: 3.0rem; text-align: center;}}
    .right {{ float: right; width: 50%; text-align: center; font-size: 2.0rem;}}
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
<div class="left3">
<label for="captureStart">Capture Start:</label></div>
<div class="left3">
<label class="switch"><input type="checkbox" onchange="toggleCheckbox(this)" id="captureStart" {} >
<span class="slider"></span></label>
</div>
<div class="clear">
<div class="left">
<label for="resolutionSelect">Camera Resolution:</label></div>
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
<label for="leapdate">Leap Date Time:</label></div>
<div class="left">
<input type="date" id="leapdate" value="2024-01-01">
<input type="time" id="leaptime" value="00:00:00">
</div></div>

<h4>Save Configuration</h4>
<h5>Timezone</h5>
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
</select>
<h5>Idle in Sleep Time</h5>
<input type="number" id="idlesleep" value="300">
<h5>Auto Capture Mode</h5>
<select id="autocaptureSelect">
<option value="true">True</option>
<option value="false">False</option>
</select>
<h5>OpenAI Model</h5>
<select id="openaiSelect">
<option value="gpt-4o">GPT-4o</option>
<option value="gpt-4-turbo">GPT-4-Turbo</option>
</select>

<h5>Config Save</h5>
<button onclick="saveConfig()">Save</button>
</div>

<script>
function saveConfig() {{
    var resolution_element = document.getElementById("resolutionSelect");
    var trackid_element = document.getElementById("trackidSelect");
    var duration_element = document.getElementById("durationSelect");
    var timezone_element = document.getElementById("timezoneSelect");
    var idlesleep_element = document.getElementById("idlesleep");
    var autocapture_element = document.getElementById("autocaptureSelect");
    var queryopenai_element = document.getElementById("queryopenai");
    var queryprompt_element = document.getElementById("queryprompt");
    var openai_element = document.getElementById("openaiSelect");
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
        "queryopenai": queryopenai_element.value,
        "queryprompt": queryprompt_element.value,
        "openai_model": openai_element.value
    }}));
}}

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
    var leapdate_element = document.getElementById("leapdate");
    var leaptime_element = document.getElementById("leaptime");
    var xhr = new XMLHttpRequest();
    xhr.open("POST", "/capture", true);
    xhr.setRequestHeader("Content-Type", "application/json");
    xhr.send(JSON.stringify({{
        "request": "start",
        "resolution": resolution_element.value,
        "trackid":  trackid_element.value - 0,
        "duration": duration_element.value - 0,
        "leaptime": {{
            "year": leapdate_element.value.substring(0, 4) - 0,
            "month": leapdate_element.value.substring(5, 7) - 0,
            "day": leapdate_element.value.substring(8, 10) - 0,
            "hour": leaptime_element.value.substring(0, 2) - 0,
            "minute": leaptime_element.value.substring(3, 5) - 0,
            "second": leaptime_element.value.substring(6, 8) - 0
        }}
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
            "year": 0,
            "month": 0,
            "day": 0,
            "hour": 0,
            "minute": 0,
            "second": 0
        }}
    }}));
}}
setInterval(function ( ) {{
    var xhttp = new XMLHttpRequest();
    xhttp.onreadystatechange = function() {{
        if (this.readyState == 4 && this.status == 200) {{
            var status = JSON.parse(this.responseText);
            var camState = "Connected";
            if( status.state == "start"){{ 
                document.getElementById("captureStart").checked = true;
            }}
            else {{ 
                document.getElementById("captureStart").checked = false;
            }}
            document.getElementById("camState").innerHTML = camState;
            // read battery voltage and wifi rssi
            var batteryVoltage = status.battery_voltage;
            var wifiRSSI = status.rssi;
            document.getElementById("batteryVoltage").innerHTML = batteryVoltage+"V";
            document.getElementById("wifiRSSI").innerHTML = wifiRSSI+"dBm";
            document.getElementById("captureID").innerHTML = status.capture_id;
            document.getElementById("lastCaptureDateTime").innerHTML = status.last_capture_date_time;
        }}
        else if (this.readyState == 4 && this.status == 0) {{
            document.getElementById("camState").innerHTML = "Not Connected";
        }}
    }};
    xhttp.open("GET", "/state", true);
    xhttp.send();
}}, 10000 ) ;

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
            document.getElementById("queryopenai").value = config.queryopenai;
            document.getElementById("queryprompt").value = config.queryprompt;
            document.getElementById("openaiSelect").value = config.openai_model;
        }}
    }};
    xhttp.open("GET", "/config", true);
    xhttp.send();
}};

// function drawAllThumbnail() {{
//     for (var i = 0; i < 11; i++) {{
//         drawThumbnail(i, "canvas" + i);
//     }}
// }}

// setInterval(function ( ) {{
//     drawAllThumbnail();
// }}, 10000 ) ;
getConfig();
// drawAllThumbnail();

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
    .left3 {{ float: left; width: 50%; font-size: 3.0rem; text-align: center;}}
    .right {{ float: right; width: 50%; text-align: center; font-size: 2.0rem;}}
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
<div class="left"><span id="wifiRSSI"><span>dBm</div>
</div>

<div class="clear">
<div class="left">
<label for="captureID">Capture ID:</label></div>
<div class="left"><span id="captureID"><span></div>
</div>

<div class="clear">
<div class="left">
<label for="lastCaptureDateTime">Last Capture Date & Time:</label></div>
<div class="left"><span id="lastCaptureDateTime"><span></div>
</div>
</div>

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

