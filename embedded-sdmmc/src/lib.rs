//! # embedded-sdmmc
//!
//! > An SD/MMC Library written in Embedded Rust
//!
//! This crate is intended to allow you to read/write files on a FAT formatted
//! SD card on your Rust Embedded device, as easily as using the `SdFat` Arduino
//! library. It is written in pure-Rust, is `#![no_std]` and does not use
//! `alloc` or `collections` to keep the memory footprint low. In the first
//! instance it is designed for readability and simplicity over performance.
//!
//! ## Using the crate
//!
//! You will need something that implements the `BlockDevice` trait, which can
//! read and write the 512-byte blocks (or sectors) from your card. If you were
//! to implement this over USB Mass Storage, there's no reason this crate
//! couldn't work with a USB Thumb Drive, but we only supply a `BlockDevice`
//! suitable for reading SD and SDHC cards over SPI.
//!
//! In particular, that means the card must be put into SPI mode separately.
//! See [`SdCard`] documentation.
//!
//! ```rust
//! use embedded_sdmmc::{Error, Mode, SdCard, SdCardError, TimeSource, VolumeIdx, VolumeManager};
//!
//! fn example<S, D, T>(spi: S, delay: D, ts: T) -> Result<(), Error<SdCardError>>
//! where
//!     S: embedded_hal::spi::SpiDevice,
//!     D: embedded_hal::delay::DelayNs,
//!     T: TimeSource,
//! {
//!     let sdcard = SdCard::new(spi, delay);
//!     println!("Card size is {} bytes", sdcard.num_bytes()?);
//!     let volume_mgr = VolumeManager::new(sdcard, ts);
//!     let volume0 = volume_mgr.open_volume(VolumeIdx(0))?;
//!     println!("Volume 0: {:?}", volume0);
//!     let root_dir = volume0.open_root_dir()?;
//!     let mut my_file = root_dir.open_file_in_dir("MY_FILE.TXT", Mode::ReadOnly)?;
//!     while !my_file.is_eof() {
//!         let mut buffer = [0u8; 32];
//!         let num_read = my_file.read(&mut buffer)?;
//!         for b in &buffer[0..num_read] {
//!             print!("{}", *b as char);
//!         }
//!     }
//!     Ok(())
//! }
//! ```
//!
//! For writing files:
//!
//! ```rust
//! use embedded_sdmmc::{BlockDevice, Directory, Error, Mode, TimeSource};
//! fn write_file<D: BlockDevice, T: TimeSource, const DIRS: usize, const FILES: usize, const VOLUMES: usize>(
//!     root_dir: &mut Directory<D, T, DIRS, FILES, VOLUMES>,
//! ) -> Result<(), Error<D::Error>>
//! {
//!     let my_other_file = root_dir.open_file_in_dir("MY_DATA.CSV", Mode::ReadWriteCreateOrAppend)?;
//!     my_other_file.write(b"Timestamp,Signal,Value\n")?;
//!     my_other_file.write(b"2025-01-01T00:00:00Z,TEMP,25.0\n")?;
//!     my_other_file.write(b"2025-01-01T00:00:01Z,TEMP,25.1\n")?;
//!     my_other_file.write(b"2025-01-01T00:00:02Z,TEMP,25.2\n")?;
//!     // Don't forget to flush the file so that the directory entry is updated
//!     my_other_file.flush()?;
//!     Ok(())
//! }
//! ```
//!
//! ## Features
//!
//! * `log`: Enabled by default. Generates log messages using the `log` crate.
//! * `defmt-log`: By turning off the default features and enabling the
//!   `defmt-log` feature you can configure this crate to log messages over defmt
//!   instead.
//!
//! You cannot enable both the `log` feature and the `defmt-log` feature.

#![cfg_attr(not(test), no_std)]
#![deny(missing_docs)]

// ****************************************************************************
//
// Imports
//
// ****************************************************************************
#[macro_use]
mod structure;

/// Re-export the tyoes library.
pub use embedded_sdmmc_types;
pub use embedded_sdmmc_types::blockdevice;
pub mod fat;
pub mod filesystem;

use core::fmt::Debug;
use embedded_io::ErrorKind;
use filesystem::Handle;

#[doc(inline)]
pub use crate::blockdevice::{Block, BlockCount, BlockDevice, BlockIdx};

#[doc(inline)]
pub use crate::fat::{FatVolume, VolumeName};

#[doc(inline)]
pub use crate::filesystem::{
    Attributes, ClusterId, DirEntry, FilenameError, LfnBuffer, MAX_FILE_SIZE, Mode, RawDirectory,
    RawFile, ShortFileName, TimeSource, Timestamp,
};

use filesystem::DirectoryInfo;

mod volume_mgr;
#[doc(inline)]
pub use volume_mgr::VolumeManager;

#[cfg(all(feature = "defmt-log", feature = "log"))]
compile_error!("Cannot enable both log and defmt-log");

#[cfg(feature = "log")]
use log::{debug, trace, warn};

#[cfg(feature = "defmt-log")]
use defmt::{debug, trace, warn};

#[cfg(all(not(feature = "defmt-log"), not(feature = "log")))]
#[macro_export]
/// Like log::debug! but does nothing at all
macro_rules! debug {
    ($($arg:tt)+) => {};
}

#[cfg(all(not(feature = "defmt-log"), not(feature = "log")))]
#[macro_export]
/// Like log::trace! but does nothing at all
macro_rules! trace {
    ($($arg:tt)+) => {};
}

#[cfg(all(not(feature = "defmt-log"), not(feature = "log")))]
#[macro_export]
/// Like log::warn! but does nothing at all
macro_rules! warn {
    ($($arg:tt)+) => {};
}

// ****************************************************************************
//
// Public Types
//
// ****************************************************************************

/// All the ways the functions in this crate can fail.
#[cfg_attr(feature = "defmt-log", derive(defmt::Format))]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error<E>
where
    E: core::error::Error,
{
    /// The underlying block device threw an error.
    #[error("error from underlying block device: {0}")]
    DeviceError(#[from] E),
    /// The filesystem is badly formatted (or this code is buggy).
    #[error("filesystem is badly formatted: {0}")]
    FormatError(&'static str),
    /// The given `VolumeIdx` was bad,
    #[error("no such volume")]
    NoSuchVolume,
    /// The given filename was bad
    #[error("bad filename")]
    FilenameError(FilenameError),
    /// Out of memory opening volumes
    #[error("too many open volumes")]
    TooManyOpenVolumes,
    /// Out of memory opening directories
    #[error("too many open directories")]
    TooManyOpenDirs,
    /// Out of memory opening files
    #[error("too many open files")]
    TooManyOpenFiles,
    /// Bad handle given
    #[error("bad handle")]
    BadHandle,
    /// That file or directory doesn't exist
    #[error("file or directory does not exist")]
    NotFound,
    /// You can't open a file twice or delete an open file
    #[error("file already open")]
    FileAlreadyOpen,
    /// You can't open a directory twice
    #[error("directory already open")]
    DirAlreadyOpen,
    /// You can't open a directory as a file
    #[error("cannot open directory as file")]
    OpenedDirAsFile,
    /// You can't open a file as a directory
    #[error("cannot open file as directory")]
    OpenedFileAsDir,
    /// You can't delete a non-empty directory
    #[error("cannot delete a non-empty directory")]
    DeleteNonEmptyDir,
    /// You can't close a volume with open files or directories
    #[error("volume is still in use")]
    VolumeStillInUse,
    /// You can't open a volume twice
    #[error("cannot open volume twice")]
    VolumeAlreadyOpen,
    /// We can't do that yet
    #[error("unsupported operation")]
    Unsupported,
    /// Tried to read beyond end of file
    #[error("end of file")]
    EndOfFile,
    /// Found a bad cluster
    #[error("bad cluster")]
    BadCluster,
    /// Error while converting types
    #[error("type conversion failed")]
    ConversionError,
    /// The device does not have enough space for the operation
    #[error("not enough space on device")]
    NotEnoughSpace,
    /// Cluster was not properly allocated by the library
    #[error("cluster not properly allocated")]
    AllocationError,
    /// Jumped to free space during FAT traversing
    #[error("FAT chain unterminated")]
    UnterminatedFatChain,
    /// Tried to open Read-Only file with write mode
    #[error("file is read-only")]
    ReadOnly,
    /// Tried to create an existing file
    #[error("file already exists")]
    FileAlreadyExists,
    /// Bad block size - only 512 byte blocks supported
    #[error("bad block size: {0} (only 512 byte blocks supported)")]
    BadBlockSize(u16),
    /// Bad offset given when seeking
    #[error("invalid seek offset")]
    InvalidOffset,
    /// Disk is full
    #[error("disk full")]
    DiskFull,
    /// A directory with that name already exists
    #[error("directory already exists")]
    DirAlreadyExists,
    /// The filesystem tried to gain a lock whilst already locked.
    ///
    /// This is either a bug in the filesystem, or you tried to access the
    /// filesystem API from inside a directory iterator (that isn't allowed).
    #[error("already locked")]
    LockError,
}

impl<E: core::error::Error + 'static> embedded_io::Error for Error<E> {
    fn kind(&self) -> ErrorKind {
        match self {
            Error::DeviceError(_)
            | Error::FormatError(_)
            | Error::FileAlreadyOpen
            | Error::DirAlreadyOpen
            | Error::VolumeStillInUse
            | Error::VolumeAlreadyOpen
            | Error::EndOfFile
            | Error::DiskFull
            | Error::NotEnoughSpace
            | Error::AllocationError
            | Error::LockError => ErrorKind::Other,
            Error::NoSuchVolume
            | Error::FilenameError(_)
            | Error::BadHandle
            | Error::InvalidOffset => ErrorKind::InvalidInput,
            Error::TooManyOpenVolumes | Error::TooManyOpenDirs | Error::TooManyOpenFiles => {
                ErrorKind::OutOfMemory
            }
            Error::NotFound => ErrorKind::NotFound,
            Error::OpenedDirAsFile
            | Error::OpenedFileAsDir
            | Error::DeleteNonEmptyDir
            | Error::BadCluster
            | Error::ConversionError
            | Error::UnterminatedFatChain => ErrorKind::InvalidData,
            Error::Unsupported | Error::BadBlockSize(_) => ErrorKind::Unsupported,
            Error::ReadOnly => ErrorKind::PermissionDenied,
            Error::FileAlreadyExists | Error::DirAlreadyExists => ErrorKind::AlreadyExists,
        }
    }
}

/// A handle to a volume.
///
/// A volume is a partition with a filesystem within it.
///
/// Do NOT drop this object! It doesn't hold a reference to the Volume Manager
/// it was created from and the VolumeManager will think you still have the
/// volume open if you just drop it, and it won't let you open the file again.
///
/// Instead you must pass it to [`crate::VolumeManager::close_volume`] to close
/// it cleanly.
#[cfg_attr(feature = "defmt-log", derive(defmt::Format))]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RawVolume(Handle);

/// A caching layer for block devices
///
/// Caches a single block.
#[derive(Debug)]
pub struct BlockCache<D> {
    block_device: D,
    block: [Block; 1],
    block_idx: Option<BlockIdx>,
}

impl<D> BlockCache<D>
where
    D: BlockDevice,
{
    /// Create a new block cache
    pub fn new(block_device: D) -> Self {
        BlockCache {
            block_device,
            block: [Block::new()],
            block_idx: None,
        }
    }

    /// Read a block, and return a reference to it.
    pub async fn read(&mut self, block_idx: BlockIdx) -> Result<&Block, D::Error> {
        if self.block_idx != Some(block_idx) {
            self.block_idx = None;
            self.block_device.read(&mut self.block, block_idx).await?;
            self.block_idx = Some(block_idx);
        }
        Ok(&self.block[0])
    }

    /// Read a block, and return a reference to it.
    pub async fn read_mut(&mut self, block_idx: BlockIdx) -> Result<&mut Block, D::Error> {
        if self.block_idx != Some(block_idx) {
            self.block_idx = None;
            self.block_device.read(&mut self.block, block_idx).await?;
            self.block_idx = Some(block_idx);
        }
        Ok(&mut self.block[0])
    }

    /// Write back a block you read with [`Self::read_mut`] and then modified.
    pub async fn write_back(&mut self) -> Result<(), D::Error> {
        self.block_device
            .write(
                &self.block,
                self.block_idx.expect("write_back with no read"),
            )
            .await
    }

    /// Write back a block you read with [`Self::read_mut`] and then modified, but to two locations.
    ///
    /// This is useful for updating two File Allocation Tables.
    pub async fn write_back_with_duplicate(&mut self, duplicate: BlockIdx) -> Result<(), D::Error> {
        self.block_device
            .write(
                &self.block,
                self.block_idx.expect("write_back with no read"),
            )
            .await?;
        self.block_device.write(&self.block, duplicate).await?;
        Ok(())
    }

    /// Access a blank sector
    pub fn blank_mut(&mut self, block_idx: BlockIdx) -> &mut Block {
        self.block_idx = Some(block_idx);
        self.block[0].fill(0);
        &mut self.block[0]
    }

    /// Access the block device
    pub fn block_device(&mut self) -> &mut D {
        // invalidate the cache
        self.block_idx = None;
        // give them the block device
        &mut self.block_device
    }

    /// Get the block device back
    pub fn free(self) -> D {
        self.block_device
    }
}

/// Internal information about a Volume
#[cfg_attr(feature = "defmt-log", derive(defmt::Format))]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VolumeInfo {
    /// Handle for this volume.
    raw_volume: RawVolume,
    /// Which volume (i.e. partition) we opened on the disk
    idx: VolumeIdx,
    /// What kind of volume this is
    volume_type: VolumeType,
}

/// This enum holds the data for the various different types of filesystems we
/// support.
#[cfg_attr(feature = "defmt-log", derive(defmt::Format))]
#[derive(Debug, PartialEq, Eq)]
pub enum VolumeType {
    /// FAT16/FAT32 formatted volumes.
    Fat(FatVolume),
}

/// A number which identifies a volume (or partition) on a disk.
///
/// `VolumeIdx(0)` is the first primary partition on an MBR partitioned disk.
#[cfg_attr(feature = "defmt-log", derive(defmt::Format))]
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct VolumeIdx(pub usize);

/// Marker for a FAT32 partition. Sometimes also use for FAT16 formatted
/// partitions.
const PARTITION_ID_FAT32_LBA: u8 = 0x0C;
/// Marker for a FAT16 partition with LBA. Seen on a Raspberry Pi SD card.
const PARTITION_ID_FAT16_LBA: u8 = 0x0E;
/// Marker for a FAT16 partition. Seen on a card formatted with the official
/// SD-Card formatter.
const PARTITION_ID_FAT16: u8 = 0x06;
/// Marker for a FAT16 partition smaller than 32MB. Seen on the wowki simulated
/// microsd card
const PARTITION_ID_FAT16_SMALL: u8 = 0x04;
/// Marker for a FAT32 partition. What Macosx disk utility (and also SD-Card formatter?)
/// use.
const PARTITION_ID_FAT32_CHS_LBA: u8 = 0x0B;

// ****************************************************************************
//
// Unit Tests
//
// ****************************************************************************
//
// #[cfg(test)]
// mod tests;
//
// #[cfg(test)]
// mod testsa {
//     use super::*;
//
//     mod utils {
//         use super::*;
//         use std::io::prelude::*;
//
//         /// This file contains:
//         ///
//         /// ```console
//         /// $ fdisk ./disk.img
//         /// Disk: ./disk.img    geometry: 520/32/63 [1048576 sectors]
//         /// Signature: 0xAA55
//         ///          Starting       Ending
//         ///  #: id  cyl  hd sec -  cyl  hd sec [     start -       size]
//         /// ------------------------------------------------------------------------
//         ///  1: 0E    0  32  33 -   16 113  33 [      2048 -     262144] DOS FAT-16
//         ///  2: 0C   16 113  34 -   65  69   4 [    264192 -     784384] Win95 FAT32L
//         ///  3: 00    0   0   0 -    0   0   0 [         0 -          0] unused
//         ///  4: 00    0   0   0 -    0   0   0 [         0 -          0] unused
//         /// $ ls -l /Volumes/P-FAT16
//         /// total 131080
//         /// -rwxrwxrwx  1 jonathan  staff  67108864  9 Dec  2018 64MB.DAT
//         /// -rwxrwxrwx  1 jonathan  staff         0  9 Dec  2018 EMPTY.DAT
//         /// -rwxrwxrwx@ 1 jonathan  staff       258  9 Dec  2018 README.TXT
//         /// drwxrwxrwx  1 jonathan  staff      2048  9 Dec  2018 TEST
//         /// $ ls -l /Volumes/P-FAT16/TEST
//         /// total 8
//         /// -rwxrwxrwx  1 jonathan  staff  3500  9 Dec  2018 TEST.DAT
//         /// $ ls -l /Volumes/P-FAT32
//         /// total 131088
//         /// -rwxrwxrwx  1 jonathan  staff  67108864  9 Dec  2018 64MB.DAT
//         /// -rwxrwxrwx  1 jonathan  staff         0  9 Dec  2018 EMPTY.DAT
//         /// -rwxrwxrwx@ 1 jonathan  staff       258 21 Sep 09:48 README.TXT
//         /// drwxrwxrwx  1 jonathan  staff      4096  9 Dec  2018 TEST
//         /// $ ls -l /Volumes/P-FAT32/TEST
//         /// total 8
//         /// -rwxrwxrwx  1 jonathan  staff  3500  9 Dec  2018 TEST.DAT
//         /// ```
//         ///
//         /// It will unpack to a Vec that is 1048576 * 512 = 512 MiB in size.
//         pub static DISK_SOURCE: &[u8] = include_bytes!("../tests/disk.img.gz");
//
//         #[derive(Debug, thiserror::Error)]
//         #[allow(dead_code)]
//         pub enum Error {
//             /// Failed to read the source image
//             #[error("I/O error: {0}")]
//             Io(#[from] std::io::Error),
//             /// Failed to unzip the source image
//             #[error("flate2 decode error: {0}")]
//             Decode(#[from] flate2::DecompressError),
//             /// Asked for a block we don't have
//             #[error("out of bounds block index: {0:?}")]
//             OutOfBounds(BlockIdx),
//         }
//
//         /// Implements the block device traits for a chunk of bytes in RAM.
//         ///
//         /// The slice should be a multiple of `embedded_sdmmc::Block::LEN` bytes in
//         /// length. If it isn't the trailing data is discarded.
//         pub struct RamDisk<T> {
//             contents: std::cell::RefCell<T>,
//         }
//
//         impl<T> RamDisk<T> {
//             fn new(contents: T) -> RamDisk<T> {
//                 RamDisk {
//                     contents: std::cell::RefCell::new(contents),
//                 }
//             }
//         }
//
//         impl<T> BlockDevice for RamDisk<T>
//         where
//             T: AsMut<[u8]> + AsRef<[u8]>,
//         {
//             type Error = Error;
//
//             async fn read(
//                 &self,
//                 blocks: &mut [Block],
//                 start_block_idx: BlockIdx,
//             ) -> Result<(), Self::Error> {
//                 let borrow = self.contents.borrow();
//                 let contents: &[u8] = borrow.as_ref();
//                 let mut block_idx = start_block_idx;
//                 for block in blocks.iter_mut() {
//                     let start_offset = block_idx.0 as usize * Block::LEN;
//                     let end_offset = start_offset + Block::LEN;
//                     if end_offset > contents.len() {
//                         return Err(Error::OutOfBounds(block_idx));
//                     }
//                     block
//                         .as_mut_slice()
//                         .copy_from_slice(&contents[start_offset..end_offset]);
//                     block_idx.0 += 1;
//                 }
//                 Ok(())
//             }
//
//             async fn write(
//                 &self,
//                 blocks: &[Block],
//                 start_block_idx: BlockIdx,
//             ) -> Result<(), Self::Error> {
//                 let mut borrow = self.contents.borrow_mut();
//                 let contents: &mut [u8] = borrow.as_mut();
//                 let mut block_idx = start_block_idx;
//                 for block in blocks.iter() {
//                     let start_offset = block_idx.0 as usize * Block::LEN;
//                     let end_offset = start_offset + Block::LEN;
//                     if end_offset > contents.len() {
//                         return Err(Error::OutOfBounds(block_idx));
//                     }
//                     contents[start_offset..end_offset].copy_from_slice(block.as_slice());
//                     block_idx.0 += 1;
//                 }
//                 Ok(())
//             }
//
//             async fn num_blocks(&self) -> Result<BlockCount, Self::Error> {
//                 let borrow = self.contents.borrow();
//                 let contents: &[u8] = borrow.as_ref();
//                 let len_blocks = contents.len() / Block::LEN;
//                 if len_blocks > u32::MAX as usize {
//                     panic!("Test disk too large! Only 2**32 blocks allowed");
//                 }
//                 Ok(BlockCount(len_blocks as u32))
//             }
//         }
//
//         /// Unpack the fixed, static, disk image.
//         fn unpack_disk(gzip_bytes: &[u8]) -> Result<Vec<u8>, Error> {
//             let disk_cursor = std::io::Cursor::new(gzip_bytes);
//             let mut gz_decoder = flate2::read::GzDecoder::new(disk_cursor);
//             let mut output = Vec::with_capacity(512 * 1024 * 1024);
//             gz_decoder.read_to_end(&mut output)?;
//             Ok(output)
//         }
//
//         /// Turn some gzipped bytes into a block device,
//         pub fn make_block_device(gzip_bytes: &[u8]) -> Result<RamDisk<Vec<u8>>, Error> {
//             let data = unpack_disk(gzip_bytes)?;
//             Ok(RamDisk::new(data))
//         }
//
//         pub struct TestTimeSource {
//             fixed: Timestamp,
//         }
//
//         impl TimeSource for TestTimeSource {
//             fn get_timestamp(&self) -> Timestamp {
//                 self.fixed
//             }
//         }
//
//         /// Make a time source that gives a fixed time.
//         ///
//         /// It always claims to be 4 April 2003, at 13:30:05.
//         ///
//         /// This is an interesting time, because FAT will round it down to 13:30:04 due
//         /// to only have two-second resolution. Hey, Real Time Clocks were optional back
//         /// in 1981.
//         pub fn make_time_source() -> TestTimeSource {
//             TestTimeSource {
//                 fixed: Timestamp {
//                     year_since_1970: 33,
//                     zero_indexed_month: 3,
//                     zero_indexed_day: 3,
//                     hours: 13,
//                     minutes: 30,
//                     seconds: 5,
//                 },
//             }
//         }
//
//         /// Get the test time source time, as a string.
//         ///
//         /// We apply the FAT 2-second rounding here.
//         #[allow(unused)]
//         pub fn get_time_source_string() -> &'static str {
//             "2003-04-04 13:30:04"
//         }
//
//         // ****************************************************************************
//         //
//         // End Of File
//         //
//         // ****************************************************************************
//     }
//
//     #[test]
//     fn open_files() {
//         let runtime = tokio::runtime::Builder::new_multi_thread()
//             .name("test")
//             .build()
//             .expect("Failed to start a tokio runtime");
//
//         runtime.block_on(async move {
//             let time_source = utils::make_time_source();
//             let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
//             let mut volume_mgr: VolumeManager<
//                 utils::RamDisk<Vec<u8>>,
//                 utils::TestTimeSource,
//                 4,
//                 2,
//                 1,
//             > = VolumeManager::new_with_limits(disk, time_source, 0xAA00_0000);
//             let volume = volume_mgr
//                 .open_raw_volume(VolumeIdx(0))
//                 .await
//                 .expect("open volume");
//             let root_dir = volume_mgr.open_root_dir(volume).expect("open root dir");
//
//             // Open with string
//             let f = volume_mgr
//                 .open_file_in_dir(root_dir, "README.TXT", Mode::ReadWriteTruncate)
//                 .await
//                 .expect("open file");
//
//             assert!(matches!(
//                 volume_mgr
//                     .open_file_in_dir(root_dir, "README.TXT", Mode::ReadOnly)
//                     .await,
//                 Err(Error::FileAlreadyOpen)
//             ));
//
//             volume_mgr.close_file(f).await.expect("close file");
//
//             // Open with SFN
//
//             let dir_entry = volume_mgr
//                 .find_directory_entry(root_dir, "README.TXT")
//                 .await
//                 .expect("find file");
//
//             let f = volume_mgr
//                 .open_file_in_dir(root_dir, &dir_entry.name, Mode::ReadWriteCreateOrAppend)
//                 .await
//                 .expect("open file with dir entry");
//
//             assert!(matches!(
//                 volume_mgr
//                     .open_file_in_dir(root_dir, &dir_entry.name, Mode::ReadOnly)
//                     .await,
//                 Err(Error::FileAlreadyOpen)
//             ));
//
//             // Can still spot duplicates even if name given the other way around
//
//             assert!(matches!(
//                 volume_mgr
//                     .open_file_in_dir(root_dir, "README.TXT", Mode::ReadOnly)
//                     .await,
//                 Err(Error::FileAlreadyOpen)
//             ));
//
//             let f2 = volume_mgr
//                 .open_file_in_dir(root_dir, "64MB.DAT", Mode::ReadWriteTruncate)
//                 .await
//                 .expect("open file");
//
//             // Hit file limit
//
//             assert!(matches!(
//                 volume_mgr
//                     .open_file_in_dir(root_dir, "EMPTY.DAT", Mode::ReadOnly)
//                     .await,
//                 Err(Error::TooManyOpenFiles)
//             ));
//
//             volume_mgr.close_file(f).await.expect("close file");
//             volume_mgr.close_file(f2).await.expect("close file");
//
//             // File not found
//
//             assert!(matches!(
//                 volume_mgr
//                     .open_file_in_dir(root_dir, "README.TXS", Mode::ReadOnly)
//                     .await,
//                 Err(Error::NotFound)
//             ));
//
//             // Create a new file
//             let f3 = volume_mgr
//                 .open_file_in_dir(root_dir, "NEWFILE.DAT", Mode::ReadWriteCreate)
//                 .await
//                 .expect("open file");
//
//             volume_mgr.write(f3, b"12345").await.expect("write to file");
//             volume_mgr.write(f3, b"67890").await.expect("write to file");
//             volume_mgr.close_file(f3).await.expect("close file");
//
//             // Open our new file
//             let f3 = volume_mgr
//                 .open_file_in_dir(root_dir, "NEWFILE.DAT", Mode::ReadOnly)
//                 .await
//                 .expect("open file");
//             // Should have 10 bytes in it
//             assert_eq!(volume_mgr.file_length(f3).expect("file length"), 10);
//             volume_mgr.close_file(f3).await.expect("close file");
//
//             volume_mgr.close_dir(root_dir).expect("close dir");
//             volume_mgr.close_volume(volume).await.expect("close volume");
//         });
//     }
// }

// ****************************************************************************
//
// End Of File
//
// ****************************************************************************
