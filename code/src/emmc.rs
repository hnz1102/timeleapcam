#![allow(dead_code)]

use log::info;
use std::ffi::c_void;
use esp_idf_sys::sdmmc_slot_config_t;
use esp_idf_sys::sdmmc_card_t;
use esp_idf_sys::sdmmc_host_t;
use esp_idf_hal::sys::{sdspi_device_config_t, spi_bus_config_t};
use anyhow::Result;

const SDMMC_HOST_SLOT_0 : i32 = 0;
const SDMMC_HOST_SLOT_1 : i32 = 1;
const SDMMC_SLOT_CONFIG_WIDTH_4 : u8 = 4;
const SDMMC_SLOT_CONFIG_WIDTH_8 : u8 = 8;
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
const SDMMC_HOST_FLAG_4BIT: u32 = 1 << 1;
const SDMMC_HOST_FLAG_1BIT: u32 = 1 << 0;
const SDMMC_FREQ_52M: i32 = 52000;
const SDMMC_FREQ_30M: i32 = 30000;
const SDMMC_FREQ_10M: i32 = 10000;
const SDMMC_FREQ_2M: i32 = 2000;
const SDMMC_HOST_FLAG_DDR: u32 = 1 << 4;

// SDSPI HOST
const SPI_HOST_ID: i32 = 1;
const SDMMC_FREQ_DEFAULT: i32 = 20000;  // 20 MHz
const SDMMC_HOST_FLAG_SPI : u32 = 1 << 3;
const SDMMC_HOST_FLAG_DEINIT_ARG : u32 = 1 << 5;
const SDSPI_SLOT_CONFIG_CMD: i32 = 9;   // GPIO9 MOSI
const SDSPI_SLOT_CONFIG_CLK: i32 = 7;   // GPIO7 SCLK
const SDSPI_SLOT_CONFIG_D0: i32 = 8;    // GPIO8 MISO
const SDSPI_SLOT_CONFIG_CS: i32 = 21;   // GPIO21 CS

const VFS_MOUNT_ALLOC_UNIT_SIZE: usize = 32 * 1024;

const MOUNT_POINT : &[u8] = b"/eMMC\0";

pub struct SDSPIHost {
    host: *mut sdmmc_host_t,
    card: esp_idf_hal::sys::sdmmc_card_t,
    sdspi_config: sdspi_device_config_t,
    spi_bus_config: spi_bus_config_t,
}

impl SDSPIHost {
    pub fn new() -> SDSPIHost {
        SDSPIHost {
            host : Box::into_raw(Box::new(esp_idf_hal::sys::sdmmc_host_t {
                flags: SDMMC_HOST_FLAG_SPI | SDMMC_HOST_FLAG_DEINIT_ARG,
                slot: SPI_HOST_ID,
                max_freq_khz: SDMMC_FREQ_DEFAULT,        
                io_voltage: 3.3,
                init: Some(esp_idf_hal::sys::sdspi_host_init),
                set_bus_width: None,
                get_bus_width: None,
                set_bus_ddr_mode: None,
                set_card_clk: Some(esp_idf_hal::sys::sdspi_host_set_card_clk),
                set_cclk_always_on: None,
                do_transaction: Some(esp_idf_hal::sys::sdspi_host_do_transaction),
                io_int_enable: Some(esp_idf_hal::sys::sdspi_host_io_int_enable),
                io_int_wait: Some(esp_idf_hal::sys::sdspi_host_io_int_wait),
                command_timeout_ms: 0,
                get_real_freq: Some(esp_idf_hal::sys::sdspi_host_get_real_freq),
                input_delay_phase: 0,
                set_input_delay: Some(esp_idf_hal::sys::sdmmc_host_set_input_delay),
                __bindgen_anon_1: esp_idf_hal::sys::sdmmc_host_t__bindgen_ty_1 {
                    deinit_p: Some(esp_idf_hal::sys::sdspi_host_remove_device),
                },        
            })),
            card: esp_idf_hal::sys::sdmmc_card_t::default(),
            sdspi_config: esp_idf_sys::sdspi_device_config_t {
                host_id: SPI_HOST_ID as u32,
                gpio_cs: SDSPI_SLOT_CONFIG_CS,
                gpio_cd: -1,
                gpio_wp: -1,
                gpio_int: -1,
                gpio_wp_polarity: false },
            spi_bus_config: esp_idf_hal::sys::spi_bus_config_t {
                __bindgen_anon_1: esp_idf_hal::sys::spi_bus_config_t__bindgen_ty_1 {
                    mosi_io_num: SDSPI_SLOT_CONFIG_CMD,
                },
                __bindgen_anon_2: esp_idf_hal::sys::spi_bus_config_t__bindgen_ty_2 {
                    miso_io_num: SDSPI_SLOT_CONFIG_D0,
                },
                sclk_io_num: SDSPI_SLOT_CONFIG_CLK,
                __bindgen_anon_3: esp_idf_hal::sys::spi_bus_config_t__bindgen_ty_3 {
                    quadwp_io_num: -1,
                },
                __bindgen_anon_4: esp_idf_hal::sys::spi_bus_config_t__bindgen_ty_4 {
                    quadhd_io_num: -1,
                },
                data4_io_num: -1,
                data5_io_num: -1,
                data6_io_num: -1,
                data7_io_num: -1,
                max_transfer_sz: 4000,
                flags: 0,
                intr_flags: 0,
                isr_cpu_id: 0,
            },
        }
    }

    pub fn mount(&mut self) -> Result<(), anyhow::Error> {       
        let slot_config_ptr = &self.sdspi_config as *const sdspi_device_config_t;
        let mount_config = esp_idf_sys::esp_vfs_fat_sdmmc_mount_config_t {
            format_if_mount_failed: true,
            max_files: 5,
            allocation_unit_size: VFS_MOUNT_ALLOC_UNIT_SIZE,
            disk_status_check_enable: false,
        };

        let mut card_ptr = &mut self.card as *mut esp_idf_hal::sys::sdmmc_card_t;

        let mount_emmc = unsafe {
            let ret = esp_idf_hal::sys::spi_bus_initialize(
                SPI_HOST_ID as u32,
                &mut self.spi_bus_config,
                esp_idf_hal::sys::spi_common_dma_t_SPI_DMA_CH_AUTO
            );
            if ret != esp_idf_sys::ESP_OK {
                return Err(anyhow::anyhow!("Failed to initialize SPI bus"));
            }
            info!("spi_bus_initialize SPIxID: {}", SPI_HOST_ID);
            esp_idf_hal::sys::esp_vfs_fat_sdspi_mount(
                MOUNT_POINT.as_ptr() as *const i8,
                self.host,
                slot_config_ptr,
                &mount_config,
                &mut card_ptr,
            )
        };
        let mount_point_str = std::str::from_utf8(MOUNT_POINT).unwrap();
        match mount_emmc {
            esp_idf_sys::ESP_OK => {
                info!("SDSPI SD Card mounted successfully {:?}", mount_point_str);
                Ok(())
            }
            _ => {
                Err(anyhow::anyhow!("Failed to mount. {:?}", mount_point_str))
            }
        }
    }
    
    #[allow(dead_code)]
    pub fn format(&mut self) {
        info!("esp_vfs_fat_sdcard_format");
        let card_ptr = &mut self.card as *mut esp_idf_hal::sys::sdmmc_card_t;
        let format_emmc = unsafe {
            esp_idf_sys::esp_vfs_fat_sdcard_format(
                MOUNT_POINT.as_ptr() as *const i8,
                card_ptr,
            )
        };
        match format_emmc {
            esp_idf_sys::ESP_OK => {
                info!("SD card formatted successfully");
            }
            _ => {
                info!("Failed to format eMMC");
            }
        }
    }
}

pub struct EMMCHost {
    host: *mut sdmmc_host_t,
    card: *mut sdmmc_card_t,
    slot_config: sdmmc_slot_config_t,
}

impl EMMCHost {
    pub fn new() -> EMMCHost {
        EMMCHost {
            host : Box::into_raw(Box::new(esp_idf_sys::sdmmc_host_t {
                // flags: SDMMC_HOST_FLAG_4BIT | SDMMC_HOST_FLAG_DDR,
                flags: SDMMC_HOST_FLAG_8BIT, // SDR
                slot: SDMMC_HOST_SLOT_0,
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
                width: SDMMC_SLOT_CONFIG_WIDTH_8,
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

    pub fn mount(&mut self) -> Result<(), anyhow::Error> {
        let slot_config_ptr = &self.slot_config as *const sdmmc_slot_config_t;
        let mount_config = esp_idf_sys::esp_vfs_fat_sdmmc_mount_config_t {
            format_if_mount_failed: true,
            max_files: 5,
            allocation_unit_size: VFS_MOUNT_ALLOC_UNIT_SIZE,
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

        let get_bus_width = unsafe {
            (*self.host).get_bus_width.unwrap()(0)
        };
        let get_clock = unsafe {
            let mut freq = 0;
            (*self.host).get_real_freq.unwrap()(0, &mut freq);
            freq
        };
        info!("Bus width: {} Freq: {} KHz", get_bus_width, get_clock);
        let mount_point_str = std::str::from_utf8(MOUNT_POINT).unwrap().replace("\0", "");
        match mount_emmc {
            esp_idf_sys::ESP_OK => {
                info!("eMMC/SD card mounted successfully {}", mount_point_str);
                Ok(())
            }
            _ => {
                Err(anyhow::anyhow!("Failed to mount {}", mount_point_str))
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
                info!("eMMC/SD card formatted successfully");
            }
            _ => {
                info!("Failed to format eMMC/SD card");
            }
        }
    }
}