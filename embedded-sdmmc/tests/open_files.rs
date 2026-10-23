//! File opening related tests

use embedded_sdmmc::{Error, Mode, VolumeIdx, VolumeManager};

mod utils;

#[tokio::test]
async fn open_files() {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let mut volume_mgr: VolumeManager<utils::RamDisk<Vec<u8>>, utils::TestTimeSource, 4, 2, 1> =
        VolumeManager::new_with_limits(disk, time_source, 0xAA00_0000);
    let volume = volume_mgr
        .open_raw_volume(VolumeIdx(0))
        .await
        .expect("open volume");
    let root_dir = volume_mgr.open_root_dir(volume).expect("open root dir");

    // Open with string
    let f = volume_mgr
        .open_file_in_dir(root_dir, "README.TXT", Mode::ReadWriteTruncate)
        .await
        .expect("open file");

    assert!(matches!(
        volume_mgr
            .open_file_in_dir(root_dir, "README.TXT", Mode::ReadOnly)
            .await,
        Err(Error::FileAlreadyOpen)
    ));

    volume_mgr.close_file(f).await.expect("close file");

    // Open with SFN

    let dir_entry = volume_mgr
        .find_directory_entry(root_dir, "README.TXT")
        .await
        .expect("find file");

    let f = volume_mgr
        .open_file_in_dir(root_dir, &dir_entry.name, Mode::ReadWriteCreateOrAppend)
        .await
        .expect("open file with dir entry");

    assert!(matches!(
        volume_mgr
            .open_file_in_dir(root_dir, &dir_entry.name, Mode::ReadOnly)
            .await,
        Err(Error::FileAlreadyOpen)
    ));

    // Can still spot duplicates even if name given the other way around

    assert!(matches!(
        volume_mgr
            .open_file_in_dir(root_dir, "README.TXT", Mode::ReadOnly)
            .await,
        Err(Error::FileAlreadyOpen)
    ));

    let f2 = volume_mgr
        .open_file_in_dir(root_dir, "64MB.DAT", Mode::ReadWriteTruncate)
        .await
        .expect("open file");

    // Hit file limit

    assert!(matches!(
        volume_mgr
            .open_file_in_dir(root_dir, "EMPTY.DAT", Mode::ReadOnly)
            .await,
        Err(Error::TooManyOpenFiles)
    ));

    volume_mgr.close_file(f).await.expect("close file");
    volume_mgr.close_file(f2).await.expect("close file");

    // File not found

    assert!(matches!(
        volume_mgr
            .open_file_in_dir(root_dir, "README.TXS", Mode::ReadOnly)
            .await,
        Err(Error::NotFound)
    ));

    // Create a new file
    let f3 = volume_mgr
        .open_file_in_dir(root_dir, "NEWFILE.DAT", Mode::ReadWriteCreate)
        .await
        .expect("open file");

    volume_mgr.write(f3, b"12345").await.expect("write to file");
    volume_mgr.write(f3, b"67890").await.expect("write to file");
    volume_mgr.close_file(f3).await.expect("close file");

    // Open our new file
    let f3 = volume_mgr
        .open_file_in_dir(root_dir, "NEWFILE.DAT", Mode::ReadOnly)
        .await
        .expect("open file");
    // Should have 10 bytes in it
    assert_eq!(volume_mgr.file_length(f3).expect("file length"), 10);
    volume_mgr.close_file(f3).await.expect("close file");

    volume_mgr.close_dir(root_dir).expect("close dir");
    volume_mgr.close_volume(volume).await.expect("close volume");
}

#[tokio::test]
async fn open_lfn() {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let mut volume_mgr: VolumeManager<utils::RamDisk<Vec<u8>>, utils::TestTimeSource, 4, 2, 1> =
        VolumeManager::new_with_limits(disk, time_source, 0xAA00_0000);
    let volume = volume_mgr
        .open_raw_volume(VolumeIdx(1))
        .await
        .expect("open volume");
    let root_dir = volume_mgr.open_root_dir(volume).expect("open root dir");
    let f = volume_mgr
        .open_long_name_file_in_dir(root_dir, "Copy of Readme.txt", Mode::ReadOnly)
        .await
        .expect("open file");

    for name in [
        "Copy of Readme.tx",
        "opy of Readme.txt",
        "Copyof Readme.txt",
        "Copy_of Readme.txt",
        "Nonsense",
    ] {
        assert!(matches!(
            volume_mgr
                .open_long_name_file_in_dir(root_dir, name, Mode::ReadOnly)
                .await,
            Err(Error::NotFound)
        ));
    }

    volume_mgr.close_file(f).await.expect("close file");
}

#[tokio::test]
async fn open_and_seek() {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let mut volume_mgr: VolumeManager<utils::RamDisk<Vec<u8>>, utils::TestTimeSource, 4, 2, 1> =
        VolumeManager::new_with_limits(disk, time_source, 0xAA00_0000);
    let volume = volume_mgr
        .open_raw_volume(VolumeIdx(0))
        .await
        .expect("open volume");
    let root_dir = volume_mgr.open_root_dir(volume).expect("open root dir");
    let f = volume_mgr
        .open_file_in_dir(root_dir, "README.TXT", Mode::ReadOnly)
        .await
        .expect("open file");

    let mut buffer = [0u8; 512];
    let len = volume_mgr
        .read(f, &mut buffer)
        .await
        .expect("read from file");
    // See directory listing in utils.rs, to see that README.TXT is 258 bytes long
    assert_eq!(len, 258);
    assert_eq!(volume_mgr.file_length(f).unwrap(), 258);
    volume_mgr.file_seek_from_current(f, 0).unwrap();
    assert!(volume_mgr.file_eof(f).unwrap());
    assert_eq!(volume_mgr.file_offset(f).unwrap(), 258);
    assert!(matches!(
        volume_mgr.file_seek_from_current(f, 1),
        Err(Error::InvalidOffset)
    ));
    volume_mgr.file_seek_from_current(f, -258).unwrap();
    assert!(!volume_mgr.file_eof(f).unwrap());
    assert_eq!(volume_mgr.file_offset(f).unwrap(), 0);
    volume_mgr.file_seek_from_current(f, 10).unwrap();
    assert!(!volume_mgr.file_eof(f).unwrap());
    assert_eq!(volume_mgr.file_offset(f).unwrap(), 10);
    volume_mgr.file_seek_from_end(f, 0).unwrap();
    assert!(volume_mgr.file_eof(f).unwrap());
    assert_eq!(volume_mgr.file_offset(f).unwrap(), 258);
    assert!(matches!(
        volume_mgr.file_seek_from_current(f, -259),
        Err(Error::InvalidOffset)
    ));
    volume_mgr.file_seek_from_start(f, 25).unwrap();
    assert!(!volume_mgr.file_eof(f).unwrap());
    assert_eq!(volume_mgr.file_offset(f).unwrap(), 25);

    volume_mgr.close_file(f).await.expect("close file");

    let Err(Error::FileAlreadyExists) = volume_mgr
        .open_file_in_dir(root_dir, "README.TXT", Mode::ReadWriteCreate)
        .await
    else {
        panic!("Expected to file to exist");
    };
}

// ****************************************************************************
//
// End Of File
//
// ****************************************************************************
