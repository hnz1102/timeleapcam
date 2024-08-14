use log::info;
use esp_camera_rs::Camera;
use esp_idf_sys::camera;
use std::{thread, time::Duration, sync::Arc, sync::Mutex};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::autofocus::AutoFocus;
use crate::imagefiles::{ ImageFiles, OpenMode, delete_file };

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
    capturing_duration: i32, 
    open_mode: OpenMode,
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
                capturing_duration: 0,
                open_mode: OpenMode::Append,
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
            let mut autofocus = AutoFocus::new(&sensor);
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
                let request = infolk.request;
                drop(infolk);
                if request {
                    info!("Capture Start...");
                    camera.return_all_framebuffers();
                    thread::sleep(Duration::from_millis(1000));
                    let mut infolk = info.lock().unwrap();
                    if infolk.wait_focus {
                        autofocus.autofocus();
                        let _autofocus_result = autofocus.get_focus_result();
                    }
                    let mut loop_count = 0;
                    infolk.status = false;
                    let filename = format!("{}/T{}/capture.dat", infolk.capture_dir, infolk.track_id);
                    let mode = match infolk.open_mode {
                        OpenMode::Append => OpenMode::Append,
                        OpenMode::Write => {
                            delete_file(Path::new(&filename));
                            OpenMode::Write
                        },
                        _ => OpenMode::Append,
                    };
                    let mut imgfile = match ImageFiles::new(&filename, mode) {
                        Ok(file) => file,
                        Err(e) => {
                            info!("Failed to create file: {:?}", e);
                            drop(infolk);
                            thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                    };
                    drop(infolk);
                    let mut average_capture_time = 0;
                    let mut average_write_time = 0;
                    let mut write_data_size = 0;
                    let mut size = 0;
                    let mut width = 0;
                    let mut height = 0;
                    let mut success_count = 0;
                    let start_capture_time = SystemTime::now();
                    loop {
                        let start_capture = SystemTime::now();
                        let frame = camera.get_framebuffer();
                        let end_capture = start_capture.elapsed().unwrap().as_micros();
                        average_capture_time += end_capture;
                        match frame {
                            Some(frame) => {
                                let buffer = frame.data();
                                size = buffer.len();
                                write_data_size += size;
                                width = frame.width();
                                height = frame.height();
                                let start_write = SystemTime::now();
                                match imgfile.write_image(buffer) {
                                    Ok(_) => {
                                        // info!("File written successfully");
                                        success_count += 1;
                                    }
                                    Err(e) => {
                                        info!("Failed to write file: {:?}", e);
                                        break;
                                    }
                                }
                                let end_write = start_write.elapsed().unwrap().as_micros();
                                average_write_time += end_write;
                                loop_count += 1;
                                camera.return_framebuffer(frame);
                                let infolk_loop = info.lock().unwrap();
                                if infolk_loop.capturing_duration > 0 {
                                    let elapsed = start_capture_time.elapsed().unwrap().as_secs();
                                    if elapsed >= infolk_loop.capturing_duration as u64 {
                                        let _ = imgfile.write_image_end();
                                        break;
                                    }
                                }
                                else if infolk_loop.capturing_duration < 0 {
                                    // infinite
                                    // capture until capturing_duration is set to 0, therefore we need to check it as latest as possible
                                }
                                else {
                                    // only one frame
                                    let _ = imgfile.write_image_end();
                                    break;
                                }
                            }
                            None => {
                                info!("No frame");
                                break;
                            }
                        }
                        if loop_count % 100 == 0 {
                            let capture_duration = start_capture_time.elapsed().unwrap().as_micros();
                            info!("Capture Duration: {}/{} {}fps {:.3}MB/S",
                                success_count, loop_count, 
                                loop_count as u64 * 1000000 / capture_duration as u64,
                                write_data_size as f32 / capture_duration as f32);
                        }
                    }
                    imgfile.flush();
                    let write_images = imgfile.get_nof_images();
                    drop(imgfile);
                    let capture_duration = start_capture_time.elapsed().unwrap().as_micros();
                    info!("Capture Frames: {} Total Frames: {} Duration:{}us {}fps {}KB", 
                        loop_count, write_images, capture_duration,
                        loop_count as u64 * 1000000 / capture_duration as u64,
                        write_data_size / 1024);
                    if loop_count > 0 {
                        info!("Average Capture Time:{}us", average_capture_time as u64 / loop_count as u64);
                        info!("Average Write Time:{}us", average_write_time as u64 / loop_count as u64);
                    }
                    let mut infolk = info.lock().unwrap();
                    infolk.capture_id = if write_images > 0 { write_images - 1 } else { 0 };
                    infolk.size = size;
                    infolk.width = width;
                    infolk.height = height;
                    infolk.status = true;
                    infolk.request = false;
                    drop(infolk);
                }
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

    pub fn set_capturing_duration(&self, duration: i32) {
        let mut info = self.info.lock().unwrap();
        info.capturing_duration = duration;
    }

    pub fn set_overwrite_saved(&self, overwrite: bool) {
        let mut info = self.info.lock().unwrap();
        let mode = if overwrite {
            OpenMode::Write
        } else {
            OpenMode::Append
        };
        info.open_mode = mode;
    }

    pub fn get_capture_id(&self) -> u32 {
        let info = self.info.lock().unwrap();
        info.capture_id
    }
}
