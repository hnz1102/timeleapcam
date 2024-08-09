use anyhow;
use log::info;
use std::fs;
use std::path::Path;
use std::io::{Write, Read, Seek};

const IMAGE_HEADER_SIZE: usize = 8;

#[derive(Debug, PartialEq, Clone)]
pub enum OpenMode {
    Read,
    Write,
    Append,
}

pub struct ImageFiles {
    file: fs::File,
    nimages: u32, 
    read_pos: u64,
    write_pos: u64,
    total_size: u64,
}

const FILE_HEADER: [u8; 16] = ['T' as u8, 'C' as u8, 'A' as u8, 'M' as u8,
                               0, 0, 0, 0,                  // Total number of images
                               0, 0, 0, 0, 0, 0, 0, 0];     // Total size of images

impl ImageFiles {
    pub fn new(directory: impl AsRef<Path>, mode: OpenMode) -> Result<ImageFiles, anyhow::Error> {
        let file = match mode {
            // Open the file for writing only
            OpenMode::Write => fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(directory.as_ref()),
            OpenMode::Append => fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(directory.as_ref()),
            OpenMode::Read => fs::OpenOptions::new()
                .read(true)
                .create(false)
                .open(directory.as_ref()), 
        };
        match file {
            Ok(mut file) => {
                let mut header : [u8; 16] = [0; 16];
                let _ = file.read(&mut header);
                let total_size : u64;
                let mut nimages : u32 = 0;
                if mode == OpenMode::Write {
                    // renew the file header
                    let _ = file.seek(std::io::SeekFrom::Start(0));
                    let _ = file.write(&FILE_HEADER);
                    total_size = 0;
                }
                else {
                    let mut renew_header = false;
                    for i in 0..4 {
                        if header[i] != FILE_HEADER[i] {
                            if mode == OpenMode::Read {
                                info!("Invalid file header: {:?}", header);
                                return Err(anyhow::Error::msg("Invalid file header"));
                            }
                            info!("Invalid file header: {:?} => Rewrite Header", header);
                            let _ = file.seek(std::io::SeekFrom::Start(0));
                            let _ = file.write(&FILE_HEADER);
                            renew_header = true;
                            break;
                        }
                    }
                    total_size = match renew_header {
                        true => 0,
                        false => {
                            let size = 
                                (header[8] as u64) +
                                ((header[9] as u64) << 8) +
                                ((header[10] as u64) << 16) +
                                ((header[11] as u64) << 24) +
                                ((header[12] as u64) << 32) +
                                ((header[13] as u64) << 40) +
                                ((header[14] as u64) << 48) +
                                ((header[15] as u64) << 56);
                            size
                        },
                    };
                    nimages = match renew_header {
                        true => 0,
                        false => {
                            let n = 
                                (header[4] as u32) +
                                ((header[5] as u32) << 8) +
                                ((header[6] as u32) << 16) +
                                ((header[7] as u32) << 24);
                            n
                        },
                    };
                }
                let read_pos = FILE_HEADER.len() as u64;
                let write_pos = match mode {
                    OpenMode::Append => total_size + read_pos,
                    _ => read_pos,
                };
                Ok(ImageFiles {
                    file: file,
                    nimages: nimages,
                    read_pos: read_pos,
                    write_pos: write_pos,
                    total_size: total_size,
                })
            }
            Err(e) => {
                info!("Failed to open file: {:?}", e);
                Err(anyhow::Error::msg("Failed to open file"))
            }
        }
    }

    #[allow(dead_code)]
    pub fn read(&mut self, buffer: &mut [u8]) -> usize {
        if self.read_pos >= self.total_size + FILE_HEADER.len() as u64 {
            return 0;
        }
        self.file.seek(std::io::SeekFrom::Start(self.read_pos)).unwrap();
        let bytes_read = self.file.read(buffer).unwrap();
        self.read_pos += bytes_read as u64;
        bytes_read
    }

    #[allow(dead_code)]
    pub fn write(&mut self, buffer: &[u8]) {
        self.file.seek(std::io::SeekFrom::Start(self.write_pos)).unwrap();
        self.file.write(buffer).unwrap();
        self.write_pos += buffer.len() as u64;
    }

    #[allow(dead_code)]
    pub fn seek(&mut self, pos: u64) {
        self.read_pos = pos;
    }

    #[allow(dead_code)]
    pub fn flush(&mut self) {
        self.file.flush().unwrap();
    }

    pub fn write_image(&mut self, buffer: &[u8]) -> Result<(), anyhow::Error> {
        let size = buffer.len();
        let header = [
            'D' as u8,
            'A' as u8,
            'T' as u8,
            'A' as u8,
            (size & 0xFF) as u8,
            ((size >> 8) & 0xFF) as u8,
            ((size >> 16) & 0xFF) as u8,
            ((size >> 24) & 0xFF) as u8,
        ];
        self.file.seek(std::io::SeekFrom::Start(self.write_pos))?;
        self.file.write(&header)?;
        self.file.write(buffer)?;
        self.write_pos += (size + IMAGE_HEADER_SIZE) as u64;
        self.total_size += (size + IMAGE_HEADER_SIZE) as u64;
        self.nimages += 1;
        Ok(())
    }

    pub fn write_image_end(&mut self) -> Result<(), anyhow::Error> {
        let header = [
            'E' as u8,
            'N' as u8,
            'D' as u8,
            0 as u8,
            0 as u8,
            0 as u8,
            0 as u8,
            0 as u8,
        ];
        self.file.seek(std::io::SeekFrom::Start(self.write_pos))?;
        self.file.write(&header)?;
        self.write_pos += IMAGE_HEADER_SIZE as u64;
        // Update write position in the file header
        self.file.seek(std::io::SeekFrom::Start(4))?;
        let file_nimages_header : [u8; 4 ] = [
            (self.nimages & 0xFF) as u8,
            ((self.nimages >> 8) & 0xFF) as u8,
            ((self.nimages >> 16) & 0xFF) as u8,
            ((self.nimages >> 24) & 0xFF) as u8,
        ];
        self.file.write(&file_nimages_header)?;
        let file_size_header : [u8; 8 ] = [
            (self.total_size & 0xFF) as u8,
            ((self.total_size >> 8) & 0xFF) as u8,
            ((self.total_size >> 16) & 0xFF) as u8,
            ((self.total_size >> 24) & 0xFF) as u8,
            ((self.total_size >> 32) & 0xFF) as u8,
            ((self.total_size >> 40) & 0xFF) as u8,
            ((self.total_size >> 48) & 0xFF) as u8,
            ((self.total_size >> 56) & 0xFF) as u8,
        ]; 
        self.file.write(&file_size_header)?;
        Ok(())
    }

    pub fn get_image_size(&mut self) -> usize {
        if self.read_pos >= self.total_size + FILE_HEADER.len() as u64 {
            return 0;
        }
        let _ = self.file.seek(std::io::SeekFrom::Start(self.read_pos));
        let mut header : [u8; IMAGE_HEADER_SIZE] = [0; IMAGE_HEADER_SIZE];
        let hsize = match self.file.read(&mut header){
            Ok(hsize) => hsize,
            Err(e) => {
                info!("Failed to read header: {:?}", e);
                return 0;
            }
        };

        if hsize != IMAGE_HEADER_SIZE {
            info!("Failed to read header size: {:?}byte", hsize);
            return 0;
        }
        // Check if the header is valid
        if header[0] != 'D' as u8 ||
           header[1] != 'A' as u8 ||
           header[2] != 'T' as u8 ||
           header[3] != 'A' as u8 {
                if  header[0] != 'E' as u8 ||
                    header[1] != 'N' as u8 ||
                    header[2] != 'D' as u8 ||
                    header[3] != 0 as u8 {
                        info!("Invalid Header {:?}", header);
                }
                return 0;
        }
        // Get the size of the image
        let size =  (header[4] as usize) +
                    ((header[5] as usize) << 8) +
                    ((header[6] as usize) << 16) +
                    ((header[7] as usize) << 24);
        size
    }

    pub fn read_image(&mut self) -> Result<Vec<u8>, anyhow::Error> {
        let size = self.get_image_size();
        if size == 0 {
            return Err(anyhow::Error::msg("Failed to get image size"));
        }
        let mut buffer = vec![0; size];
        let _ = self.file.seek(std::io::SeekFrom::Start(self.read_pos + IMAGE_HEADER_SIZE as u64));
        let bytes_read = match self.file.read(&mut buffer[0..size]){
            Ok(bytes_read) => bytes_read,
            Err(e) => {
                info!("Failed to read image: {:?}", e);
                return Err(anyhow::Error::msg("Failed to read image"));
            }
        };
        self.read_pos += (bytes_read + IMAGE_HEADER_SIZE) as u64;
        Ok(buffer)
    }

    pub fn seek_image(&mut self, from_frame: u32) -> Result<(), anyhow::Error> {
        for _ in 0..from_frame {
            let size = self.get_image_size();
            if size == 0 {
                return Err(anyhow::Error::msg("Failed to get image size"));
            }
            self.read_pos += (size + IMAGE_HEADER_SIZE) as u64;
        }
        Ok(())
    }

    pub fn get_nof_images(&self) -> u32 {
        self.nimages
    }
}

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
    // info!("Writing file: {:?}", file);
    let file = fs::File::create(file);
    match file {
        Ok(mut file) => {
            match file.write(data) {
                Ok(_) => {
                    // info!("File written successfully");
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