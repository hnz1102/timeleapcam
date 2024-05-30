use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};
use std::sync::atomic::{AtomicBool, Ordering};
use std::ffi::c_void;
use log::*;

const MAX_TOUCHPADS: usize = 1;
const THRESHOLD_PERCENT: f32 = 0.02;

static TOUCH_ACTIVE_FLAG: AtomicBool = AtomicBool::new(false);

#[allow(dead_code)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Center,
}

#[derive(Debug, Clone, Copy)]
pub enum KeyEvent {
    UpKeyDown,
    UpKeyUp,
    DownKeyDown,
    DownKeyUp,
    LeftKeyDown,
    LeftKeyUp,
    RightKeyDown,
    RightKeyUp,
    CenterKeyDown,
    CenterKeyUp,
}

struct KeyState {
    up: bool,
    up_time: SystemTime,
    up_duration: u32,
    down: bool,
    down_time: SystemTime,
    down_duration: u32,
    left: bool,
    left_time: SystemTime,
    left_duration: u32,
    right: bool,
    right_time: SystemTime,
    right_duration: u32,
    center: bool,
    center_time: SystemTime,
    center_duration: u32,
    key_envet: Vec<KeyEvent>,
}

#[derive(Debug)]
#[allow(unused)]
enum TouchPadChannel {
    TouchPad1 = 1,
    // TouchPad2 = 2,
    // TouchPad3 = 3,
    // TouchPad4 = 4,
    // TouchPad5 = 5,
    // TouchPad6 = 6,
    // TouchPad7 = 7,
    // TouchPad8 = 8,
    // TouchPad9 = 9,
    // TouchPad10 = 10,
    // TouchPad11 = 11,
    // TouchPad12 = 12,
    // TouchPad13 = 13,
    // TouchPad14 = 14,
}

const USE_TOUCH_PAD_CHANNEL : [TouchPadChannel; 1] = [
    TouchPadChannel::TouchPad1,
    // TouchPadChannel::TouchPad2,
    // TouchPadChannel::TouchPad5,
    // TouchPadChannel::TouchPad6,
    // TouchPadChannel::TouchPad7,
];

struct TouchState {
    smooth_value: [u32; MAX_TOUCHPADS],
}

pub struct TouchPad {
    touch_state: Arc<Mutex<TouchState>>,
    key_state: Arc<Mutex<KeyState>>,
}

unsafe extern "C" fn touch_key_interrupt_handler(_arg: *mut c_void) {
    let intr = esp_idf_sys::touch_pad_read_intr_status_mask();
    if (intr & (esp_idf_sys::touch_pad_intr_mask_t_TOUCH_PAD_INTR_MASK_ACTIVE as u32 |
                esp_idf_sys::touch_pad_intr_mask_t_TOUCH_PAD_INTR_MASK_INACTIVE as u32)
    ) != 0 {
        TOUCH_ACTIVE_FLAG.store(true, Ordering::Relaxed);
    }
}

#[allow(dead_code)]
impl TouchPad {
    pub fn new() -> TouchPad {
        TouchPad { touch_state: Arc::new(Mutex::new(
            TouchState {
                            smooth_value: [0; MAX_TOUCHPADS],
            })),
            key_state: Arc::new(Mutex::new(
                KeyState {  up: false, up_time: SystemTime::now(), up_duration: 0,
                            down: false, down_time: SystemTime::now(), down_duration: 0,
                            left: false, left_time: SystemTime::now(), left_duration: 0,
                            right: false, right_time: SystemTime::now(), right_duration: 0,
                            center: false, center_time: SystemTime::now(), center_duration: 0,
                            key_envet: Vec::new(),
                })),
        }
    }

    pub fn start(&mut self)
    {
        let touch_state = self.touch_state.clone();
        let key_state = self.key_state.clone();
        let _th = thread::spawn(move || {
            info!("Start TouchPad Read Thread.");
            unsafe {
                esp_idf_sys::touch_pad_init();
                for i in USE_TOUCH_PAD_CHANNEL.iter() {
                    match i {
                        TouchPadChannel::TouchPad1 => {
                            esp_idf_sys::touch_pad_config(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM1);
                        },
                        // TouchPadChannel::TouchPad2 => {
                        //     esp_idf_sys::touch_pad_config(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM2);
                        // },
                        // TouchPadChannel::TouchPad5 => {
                        //     esp_idf_sys::touch_pad_config(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM5);
                        // },
                        // TouchPadChannel::TouchPad6 => {
                        //     esp_idf_sys::touch_pad_config(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM6);
                        // },
                        // TouchPadChannel::TouchPad7 => {
                        //     esp_idf_sys::touch_pad_config(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM7);
                        // },
                        // _ => {},
                    }
                }
                esp_idf_sys::touch_pad_isr_register(Some(touch_key_interrupt_handler), std::ptr::null_mut(),
                    esp_idf_sys::touch_pad_intr_mask_t_TOUCH_PAD_INTR_MASK_ACTIVE |
                    esp_idf_sys::touch_pad_intr_mask_t_TOUCH_PAD_INTR_MASK_INACTIVE);
                esp_idf_sys::touch_pad_intr_enable(
                    esp_idf_sys::touch_pad_intr_mask_t_TOUCH_PAD_INTR_MASK_ACTIVE |
                    esp_idf_sys::touch_pad_intr_mask_t_TOUCH_PAD_INTR_MASK_INACTIVE);
                esp_idf_sys::touch_pad_set_fsm_mode(esp_idf_sys::touch_fsm_mode_t_TOUCH_FSM_MODE_TIMER);
                esp_idf_sys::touch_pad_fsm_start();
                thread::sleep(Duration::from_millis(100));
                let mut lck = touch_state.lock().unwrap();
                for i in USE_TOUCH_PAD_CHANNEL.iter() {
                    match i {
                        TouchPadChannel::TouchPad1 => {
                            esp_idf_sys::touch_pad_filter_read_smooth(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM1, &mut lck.smooth_value[0]);
                            esp_idf_sys::touch_pad_set_thresh(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM1, (lck.smooth_value[0] as f32 * THRESHOLD_PERCENT) as u32);
                            info!("TouchPad1 threshold: {}", (lck.smooth_value[0] as f32 * THRESHOLD_PERCENT) as u32);
                        },
                        // TouchPadChannel::TouchPad2 => {
                        //     esp_idf_sys::touch_pad_filter_read_smooth(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM2, &mut lck.smooth_value[1]);
                        //     esp_idf_sys::touch_pad_set_thresh(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM2, (lck.smooth_value[1] as f32 * THRESHOLD_PERCENT) as u32);
                        //     info!("TouchPad2 threshold: {}", (lck.smooth_value[1] as f32 * THRESHOLD_PERCENT) as u32);
                        // },
                        // TouchPadChannel::TouchPad5 => {
                        //     esp_idf_sys::touch_pad_filter_read_smooth(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM5, &mut lck.smooth_value[4]);
                        //     esp_idf_sys::touch_pad_set_thresh(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM5, (lck.smooth_value[4] as f32 * THRESHOLD_PERCENT) as u32);
                        //     info!("TouchPad5 threshold: {}", (lck.smooth_value[4] as f32 * THRESHOLD_PERCENT) as u32);
                        // },
                        // TouchPadChannel::TouchPad6 => {
                        //     esp_idf_sys::touch_pad_filter_read_smooth(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM6, &mut lck.smooth_value[5]);
                        //     esp_idf_sys::touch_pad_set_thresh(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM6, (lck.smooth_value[5] as f32 * THRESHOLD_PERCENT) as u32);
                        //     info!("TouchPad6 threshold: {}", (lck.smooth_value[5] as f32 * THRESHOLD_PERCENT) as u32);
                        // },
                        // TouchPadChannel::TouchPad7 => {
                        //     esp_idf_sys::touch_pad_filter_read_smooth(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM7, &mut lck.smooth_value[6]);
                        //     esp_idf_sys::touch_pad_set_thresh(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM7, (lck.smooth_value[6] as f32 * THRESHOLD_PERCENT) as u32);
                        //     info!("TouchPad7 threshold: {}", (lck.smooth_value[6] as f32 * THRESHOLD_PERCENT) as u32);
                        // },
                        // _ => {},
                    }
                }
            }

            loop {
                thread::sleep(Duration::from_millis(100));
                // raw data from touch pad
                // unsafe {
                //     let mut value = 0;
                //     esp_idf_sys::touch_pad_read_raw_data(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM1, &mut value);
                //     info!("TouchPad1 raw data: {}", value);
                //     esp_idf_sys::touch_pad_read_raw_data(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM2, &mut value);
                //     info!("TouchPad2 raw data: {}", value);
                //     esp_idf_sys::touch_pad_read_raw_data(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM5, &mut value);
                //     info!("TouchPad5 raw data: {}", value);
                //     esp_idf_sys::touch_pad_read_raw_data(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM6, &mut value);
                //     info!("TouchPad6 raw data: {}", value);
                //     esp_idf_sys::touch_pad_read_raw_data(esp_idf_sys::touch_pad_t_TOUCH_PAD_NUM7, &mut value);
                //     info!("TouchPad7 raw data: {}", value);
                // }

                if TOUCH_ACTIVE_FLAG.load(Ordering::Relaxed) {
                    let mut keylck = key_state.lock().unwrap();
                    unsafe {
                        let touch_status = esp_idf_sys::touch_pad_get_status();
                        // info!("TouchPad status: {:08b}", touch_status);
                        for i in 0..=MAX_TOUCHPADS {
                            if touch_status & (1 << i) != 0 {
                                info!("TouchPad{} touched.", i);
                                match i {
                                    5 => {
                                        if ! keylck.up {
                                            keylck.up = true;
                                            keylck.up_time = SystemTime::now();
                                            keylck.key_envet.push(KeyEvent::UpKeyDown);
                                            info!("UpKeyDown");
                                        }
                                    },
                                    6 => {
                                        if ! keylck.down {
                                            keylck.down = true;
                                            keylck.down_time = SystemTime::now();
                                            keylck.key_envet.push(KeyEvent::DownKeyDown);
                                            info!("DownKeyDown");
                                        }
                                    },
                                    2 => {
                                        if ! keylck.left {
                                            keylck.left = true;
                                            keylck.left_time = SystemTime::now();
                                            keylck.key_envet.push(KeyEvent::LeftKeyDown);
                                            info!("LeftKeyDown");
                                        }
                                    },
                                    7 => {
                                        if ! keylck.right {
                                            keylck.right = true;
                                            keylck.right_time = SystemTime::now();
                                            keylck.key_envet.push(KeyEvent::RightKeyDown);
                                            info!("RightKeyDown");
                                        }
                                    },
                                    1 => {
                                        if ! keylck.center {
                                            keylck.center = true;
                                            keylck.center_time = SystemTime::now();
                                            keylck.key_envet.push(KeyEvent::CenterKeyDown);
                                            info!("CenterKeyDown");
                                        }
                                    },
                                    _ => {},
                                }
                            }
                            else {
                                match i {
                                    5 => {
                                        if keylck.up {
                                            keylck.up = false;
                                            keylck.up_duration = keylck.up_time.elapsed().unwrap().as_millis() as u32;
                                            keylck.key_envet.push(KeyEvent::UpKeyUp);
                                            info!("UpKeyUp");
                                        }
                                    },
                                    6 => {
                                        if keylck.down {
                                            keylck.down = false;
                                            keylck.down_duration = keylck.down_time.elapsed().unwrap().as_millis() as u32;
                                            keylck.key_envet.push(KeyEvent::DownKeyUp);
                                            info!("DownKeyUp");
                                        }
                                    },
                                    2 => {
                                        if keylck.left {
                                            keylck.left = false;
                                            keylck.left_duration = keylck.left_time.elapsed().unwrap().as_millis() as u32;
                                            keylck.key_envet.push(KeyEvent::LeftKeyUp);
                                            info!("LeftKeyUp");
                                        }
                                    },
                                    7 => {
                                        if keylck.right {
                                            keylck.right = false;
                                            keylck.right_duration = keylck.right_time.elapsed().unwrap().as_millis() as u32;
                                            keylck.key_envet.push(KeyEvent::RightKeyUp);
                                            info!("RightKeyUp");
                                        }
                                    },
                                    1 => {
                                        if keylck.center {
                                            keylck.center = false;
                                            keylck.center_duration = keylck.center_time.elapsed().unwrap().as_millis() as u32;
                                            keylck.key_envet.push(KeyEvent::CenterKeyUp);
                                            info!("CenterKeyUp");
                                        }
                                    },
                                    _ => {},
                                }
                            }
                        }
                    }
                    TOUCH_ACTIVE_FLAG.store(false, Ordering::Relaxed);
                }
            }
        });
    }

    pub fn get_touchpad_status(&mut self, key: Key) -> bool
    {
        let lck = self.key_state.lock().unwrap();
        match key {
            Key::Up => {
                lck.up
            },
            Key::Down => {
                lck.down
            },
            Key::Left => {
                lck.left
            },
            Key::Right => {
                lck.right
            },
            Key::Center => {
                lck.center
            },
        }
    }
    
    pub fn get_button_press_time(&mut self, key: Key) -> u32
    {
        let lck = self.key_state.lock().unwrap();
        match key {
            Key::Up => {
                lck.up_duration
            },
            Key::Down => {
                lck.down_duration
            },
            Key::Left => {
                lck.left_duration
            },
            Key::Right => {
                lck.right_duration
            },
            Key::Center => {
                lck.center_duration
            },
        }
    }

    pub fn clear_all_button_event(&mut self)
    {
        let mut lck = self.key_state.lock().unwrap();
        lck.key_envet.clear();
    }

    pub fn get_key_event_and_clear(&mut self) -> Vec<KeyEvent>
    {
        let mut lck = self.key_state.lock().unwrap();
        let ret = lck.key_envet.clone();
        lck.key_envet.clear();
        ret
    }

    pub fn stop_touchpad(&mut self)
    {
        unsafe {
            esp_idf_sys::touch_pad_intr_disable(
                esp_idf_sys::touch_pad_intr_mask_t_TOUCH_PAD_INTR_MASK_ACTIVE |
                esp_idf_sys::touch_pad_intr_mask_t_TOUCH_PAD_INTR_MASK_INACTIVE);
            esp_idf_sys::touch_pad_fsm_stop();
            esp_idf_sys::touch_pad_isr_register(None, std::ptr::null_mut(),
                esp_idf_sys::touch_pad_intr_mask_t_TOUCH_PAD_INTR_MASK_ACTIVE |
                esp_idf_sys::touch_pad_intr_mask_t_TOUCH_PAD_INTR_MASK_INACTIVE);
            // 
        }
    }
}