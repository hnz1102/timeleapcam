use log::info;
use esp_camera_rs::Camera;
use esp_idf_sys::camera;
use std::{thread, time::Duration, sync::Arc, sync::Mutex};
use std::fs;
use std::path::Path;

use crate::autofocus::AutoFocus;
use crate::imagefiles::{write_file};

#[derive(Debug, Clone)]
pub struct CaptureInfo {
    pub track_id: u32,
    pub capture_id: u32,
    pub capture_dir: String,
    request: bool,
    autofocus_request: bool,
    change_resolution: bool,
    resolution: camera::framesize_t,
    wait_focus: bool,
    pub status: bool,
    pub width: usize,
    pub height: usize,
    pub size: usize,
}

pub struct Capture {
    camera: Arc<Mutex<Camera<'static>>>,
    info: Arc<Mutex<CaptureInfo>>,
}

impl Capture {
    pub fn new(camera: Camera<'static>, dir: &str) -> Self {
        Capture {
            camera: Arc::new(Mutex::new(camera)),
            info: Arc::new(Mutex::new(CaptureInfo {
                track_id: 0,
                capture_id: 0,
                capture_dir: dir.to_string(), 
                request: false,
                autofocus_request: false,
                change_resolution: false,
                resolution: camera::framesize_t_FRAMESIZE_VGA,
                wait_focus: false,
                status: false,
                width: 0,
                height: 0,
                size: 0,
             })),
        }
    }

    pub fn start(&mut self) {
        let camera = self.camera.clone();
        let info = self.info.clone();
        let _th = thread::spawn(move || {
            info!("Capturing Frame Thread Start...");
            let camera = camera.lock().unwrap();
            let sensor = camera.sensor();
            let autofocus = AutoFocus::new(&sensor);
            autofocus.init();
            let _ = sensor.set_quality(10);
            let _ = sensor.set_hmirror(true);
            autofocus.autofocus_zoneconfig(); 
            // autofocus.autofocus();

            let mut current_status = false;
            loop {
                let mut infolk = info.lock().unwrap();
                if infolk.change_resolution {
                    let _ = sensor.set_framesize(infolk.resolution);
                    autofocus.autofocus_zoneconfig();
                    autofocus.autofocus();
                    infolk.change_resolution = false;
                }
                if current_status == false && infolk.request {
                    current_status = infolk.request;
                    // create directory
                    let dir = format!("{}/T{}", infolk.capture_dir, infolk.track_id);
                    fs::create_dir_all(&dir).expect("Failed to create directory");
                }
                if current_status && !infolk.request {
                    current_status = infolk.request;
                }
                if infolk.autofocus_request {
                    autofocus.autofocus();
                    infolk.autofocus_request = false;
                }
                if infolk.request {
                    info!("Capture Start...");
                    if infolk.wait_focus {
                        autofocus.autofocus();
                        let _autofocus_result = autofocus.get_focus_result();
                    }
                    // 1st frame is dropped
                    let frame = camera.get_framebuffer();
                    camera.return_framebuffer(frame.expect("Failed to get frame"));
                    // 2nd frame is captured
                    infolk.status = false;
                    let frame = camera.get_framebuffer();
                    match frame {
                        Some(frame) => {
                            let buffer = frame.data();
                            info!("frame width:{} height:{} image_size:{}", frame.width(), frame.height(), buffer.len());
                            infolk.width = frame.width();
                            infolk.height = frame.height();
                            infolk.size = buffer.len();
                            let filename = format!("{}/T{}/I{}.jpg", infolk.capture_dir, infolk.track_id, infolk.capture_id);
                            write_file(Path::new(&filename), buffer);
                            infolk.status = true;
                            camera.return_framebuffer(frame);
                        }
                        None => {
                            info!("No frame");
                        }
                    }
                    infolk.request = false;
                }
                drop(infolk);
                thread::sleep(Duration::from_millis(100));
            }
        });
    }

    #[allow(dead_code)]
    pub fn set_capture_dir(&self, dir: &str) {
        let mut info = self.info.lock().unwrap();
        info.capture_dir = dir.to_string();
    }

    #[allow(dead_code)]
    pub fn capture_request(&self, track_id: u32, id: u32) {
        let mut info = self.info.lock().unwrap();
        info.track_id = track_id;
        info.request = true;
        info.status = false;
        info.capture_id = id;
    }

    #[allow(dead_code)]
    pub fn get_capture_status(&self) -> bool {
        let info = self.info.lock().unwrap();
        info.status
    }

    #[allow(dead_code)]
    pub fn get_capture_info(&self) -> CaptureInfo {
        let info = self.info.lock().unwrap();
        info.clone()
    }

    #[allow(dead_code)]
    pub fn wait_focus(&self, wait: bool) {
        let mut info = self.info.lock().unwrap();
        info.wait_focus = wait;
    }

    #[allow(dead_code)]
    pub fn get_resolution(&self) -> camera::framesize_t {
        let info = self.info.lock().unwrap();
        info.resolution
    }

    #[allow(dead_code)]
    pub fn change_resolution(&self, resolution: camera::framesize_t) {
        let mut info = self.info.lock().unwrap();
        info.change_resolution = true;
        info.resolution = resolution;
    }

    #[allow(dead_code)]
    pub fn autofocus_request(&self) {
        let mut info = self.info.lock().unwrap();
        info.autofocus_request = true;
    }
}
