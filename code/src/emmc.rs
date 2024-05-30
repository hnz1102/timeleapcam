use log::info;
use std::ffi::c_void;
use esp_idf_sys::sdmmc_slot_config_t;
use esp_idf_sys::sdmmc_card_t;
use esp_idf_sys::sdmmc_host_t;

const SDMMC_SLOT_CONFIG_WIDTH : u8 = 8;
const SDMMC_SLOT_CONFIG_CLK : i32 = 21;
const SDMMC_SLOT_CONFIG_CMD : i32 = 45;
const SDMMC_SLOT_CONFIG_D0 : i32 = 4;
const SDMMC_SLOT_CONFIG_D1 : i32 = 5;
const SDMMC_SLOT_CONFIG_D2 : i32 = 6;
const SDMMC_SLOT_CONFIG_D3 : i32 = 7;
const SDMMC_SLOT_CONFIG_D4 : i32 = 8;
const SDMMC_SLOT_CONFIG_D5 : i32 = 3;
const SDMMC_SLOT_CONFIG_D6 : i32 = 46;
const SDMMC_SLOT_CONFIG_D7 : i32 = 9;
const SDMMC_HOST_FLAG_8BIT: u32 = 1 << 2;
// const SDMMC_HOST_FLAG_4BIT: u32 = 1 << 1;
// const SDMMC_HOST_FLAG_1BIT: u32 = 1 << 0;
const SDMMC_FREQ_52M: i32 = 52000;
// const SDMMC_FREQ_30M: i32 = 30000;
// const SDMMC_FREQ_10M: i32 = 10000;
// const SDMMC_FREQ_2M: i32 = 2000;

const MOUNT_POINT : &[u8] = b"/eMMC\0";

pub struct EMMCHost {
    host: *mut sdmmc_host_t,
    card: *mut sdmmc_card_t,
    slot_config: sdmmc_slot_config_t,
}

impl EMMCHost {
    pub fn new() -> EMMCHost {
        EMMCHost {
            host : Box::into_raw(Box::new(esp_idf_sys::sdmmc_host_t {
                flags: SDMMC_HOST_FLAG_8BIT,
                slot: 0,
                max_freq_khz: SDMMC_FREQ_52M,
                io_voltage: 3.3,
                init: Some(esp_idf_sys::sdmmc_host_init),
                set_bus_width: Some(esp_idf_sys::sdmmc_host_set_bus_width),
                get_bus_width: Some(esp_idf_sys::sdmmc_host_get_slot_width),
                set_bus_ddr_mode: Some(esp_idf_sys::sdmmc_host_set_bus_ddr_mode),
                set_card_clk: Some(esp_idf_sys::sdmmc_host_set_card_clk),
                set_cclk_always_on: Some(esp_idf_sys::sdmmc_host_set_cclk_always_on),
                do_transaction: Some(esp_idf_sys::sdmmc_host_do_transaction),
                io_int_enable: Some(esp_idf_sys::sdmmc_host_io_int_enable),
                io_int_wait: Some(esp_idf_sys::sdmmc_host_io_int_wait),
                command_timeout_ms: 0,
                get_real_freq: Some(esp_idf_sys::sdmmc_host_get_real_freq),
                __bindgen_anon_1: esp_idf_sys::sdmmc_host_t__bindgen_ty_1 {
                    deinit: Some(esp_idf_sys::sdmmc_host_deinit),
                },
                input_delay_phase: 0,
                set_input_delay: None,
            })),
            card: std::ptr::null_mut(),
            slot_config: esp_idf_sys::sdmmc_slot_config_t {
                width: SDMMC_SLOT_CONFIG_WIDTH,
                clk: SDMMC_SLOT_CONFIG_CLK,
                cmd: SDMMC_SLOT_CONFIG_CMD,
                d0: SDMMC_SLOT_CONFIG_D0,
                d1: SDMMC_SLOT_CONFIG_D1,
                d2: SDMMC_SLOT_CONFIG_D2,
                d3: SDMMC_SLOT_CONFIG_D3,
                d4: SDMMC_SLOT_CONFIG_D4,
                d5: SDMMC_SLOT_CONFIG_D5,
                d6: SDMMC_SLOT_CONFIG_D6,
                d7: SDMMC_SLOT_CONFIG_D7,
                __bindgen_anon_1: esp_idf_sys::sdmmc_slot_config_t__bindgen_ty_1 {
                    gpio_cd: -1,
                },
                __bindgen_anon_2: esp_idf_sys::sdmmc_slot_config_t__bindgen_ty_2 {
                    gpio_wp: -1,
                },
                flags: 0,
            },
        }
    }

    pub fn mount(&mut self) {
        let slot_config_ptr = &self.slot_config as *const sdmmc_slot_config_t;
        let mount_config = esp_idf_sys::esp_vfs_fat_sdmmc_mount_config_t {
            format_if_mount_failed: true,
            max_files: 5,
            allocation_unit_size: 512,
            disk_status_check_enable: false,
        };

        self.card = std::ptr::null_mut();
        let card_ptr = &mut self.card as *mut *mut sdmmc_card_t;

        let mount_emmc = unsafe {
            esp_idf_sys::esp_vfs_fat_sdmmc_mount(
            MOUNT_POINT.as_ptr() as *const i8,
            self.host,
            slot_config_ptr as *const c_void,
            &mount_config,
            card_ptr,
            )
        };

        match mount_emmc {
            esp_idf_sys::ESP_OK => {
                println!("eMMC mounted successfully");
            }
            _ => {
                info!("Failed to mount eMMC");
            }
        }   
    }

    #[allow(dead_code)]
    pub fn format(&self) {
        info!("esp_vfs_fat_sdcard_format");
        let format_emmc = unsafe {
            esp_idf_sys::esp_vfs_fat_sdcard_format(
                MOUNT_POINT.as_ptr() as *const i8,
                self.card,
            )
        };
        match format_emmc {
            esp_idf_sys::ESP_OK => {
                println!("eMMC formatted successfully");
            }
            _ => {
                info!("Failed to format eMMC");
            }
        }
    }
}