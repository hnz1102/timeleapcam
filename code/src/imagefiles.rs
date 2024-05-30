use log::info;
use std::fs;
use std::path::Path;
use std::io::{Write, Read};

// Read the Capture file
#[allow(dead_code)]
pub fn read_file(filepath: &Path) -> Vec<u8> {
    let file = fs::File::open(filepath);
    match file {
        Ok(mut file) => {
            let mut buffer = Vec::new();
            match file.read_to_end(&mut buffer) {
                Ok(_) => {
                    info!("File read successfully {:?} {:?} bytes", filepath, buffer.len());
                    buffer
                }
                Err(e) => {
                    info!("Failed to read file: {:?}", e);
                    Vec::new()
                }
            }
        }
        Err(e) => {
            info!("Failed to open file: {:?}", e);
            Vec::new()
        }
    }
}

// Write the Capture file
#[allow(dead_code)]
pub fn write_file(file: &Path, data: &[u8]) {
    info!("Writing file: {:?}", file);
    let file = fs::File::create(file);
    match file {
        Ok(mut file) => {
            match file.write_all(data) {
                Ok(_) => {
                    info!("File written successfully");
                }
                Err(e) => {
                    info!("Failed to write file: {:?}", e);
                }
            }
            // file.flush().unwrap();
        }
        Err(e) => {
            info!("Failed to create file: {:?}", e);
        }
    }
}

// List files in the directory
#[allow(dead_code)]
pub fn list_files(directory: &Path ) {
    // Create a file
    let file = fs::read_dir(directory);
    match file {
        Ok(file) => {
            for entry in file {
                let entry = entry.unwrap();
                info!("{:?} Len: {:?}", entry.path().display(), entry.metadata().unwrap().len());
            }        
        }
        Err(e) => {
            info!("Failed to open file: {:?}", e);
        }
    }
}

// Delete the Capture file
#[allow(dead_code)]
pub fn delete_file(file: &Path) {
    let result = fs::remove_file(file);
    match result {
        Ok(_) => {
            info!("File deleted successfully");
        }
        Err(e) => {
            info!("Failed to delete file: {:?}", e);
        }
    }
}

// Delete all files in the directory
#[allow(dead_code)]
pub fn delete_all_files(directory: &Path) {
    let result = fs::remove_dir_all(directory);
    match result {
        Ok(_) => {
            info!("Files deleted successfully");
        }
        Err(e) => {
            info!("Failed to delete files: {:?}", e);
        }
    }
}