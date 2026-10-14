#[allow(dead_code)]
#[path = "../../../../src-tauri/src/local_backup.rs"]
mod local_backup;

use local_backup::{LocalBackupError, NeverCancelled, PayloadTarget, StagedLocalBackupEntry, StrictLocalBackupDatabaseRestore};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

struct ExpectedDatabase { sha256: String, restored: bool }
impl StrictLocalBackupDatabaseRestore for ExpectedDatabase {
    fn restore_database(&mut self, entry: &StagedLocalBackupEntry, _: &[StagedLocalBackupEntry]) -> Result<(), LocalBackupError> {
        assert_eq!(entry.logical_name, "database.risudat");
        assert_eq!(entry.sha256, self.sha256);
        self.restored = true;
        Ok(())
    }
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert_eq!(args.len(), 3, "archive staging-directory database-sha256");
    let path = PathBuf::from(&args[0]);
    let staging = PathBuf::from(&args[1]);
    assert!(path.is_file());
    fs::create_dir_all(&staging).unwrap();
    let mut restore = ExpectedDatabase { sha256: args[2].clone(), restored: false };
    let report = local_backup::parse_legacy_local_backup_v1(
        &mut fs::File::open(&path).unwrap(), &staging, PayloadTarget::JobStaging, &mut restore, &NeverCancelled
    ).unwrap();
    assert!(restore.restored);
    assert_eq!(report.source_bytes, fs::metadata(&path).unwrap().len());
    for entry in &report.entries {
        let bytes = fs::read(entry.staged_path.as_ref().unwrap()).unwrap();
        assert_eq!(entry.byte_length, bytes.len() as u64);
        assert_eq!(entry.sha256, hex::encode(Sha256::digest(bytes)));
    }
    println!("Validated {} entries and {} bytes with the production container parser.", report.entries.len(), report.source_bytes);
}
