//! Messages émis par l'API vers le daemon.

pub mod backup;
pub mod content;
mod error;
pub mod file;
mod hello_ack;
pub mod install;
pub mod server;

use serde::{Deserialize, Serialize};

pub use backup::{
    BackupCreate, BackupDelete, BackupHook, BackupList, BackupRead, BackupRestore, RestoreMode,
};
pub use content::{
    ContentExtract, ContentFile, ContentHash, ContentInstall, ContentRemove, ContentToggle,
    ExtractRoute, lookup, matches,
};
pub use error::{ErrorCode, ProtocolError};
pub use file::{
    FileDelete, FileList, FileMkdir, FileRead, FileRename, FileUpload, FileUploadChunk, FileWrite,
};
pub use hello_ack::HelloAck;
pub use install::{
    InstallMode, InstallSpec, InstallStep, ServerInstall, SteamLogin, SteamPlatform,
};
pub use server::{
    ServerCommand, ServerDelete, ServerDeploy, ServerLimits, ServerPower, ServerSuspend,
    ServerSync, ServerWatch,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ApiMessage {
    #[serde(rename = "hello.ack")]
    HelloAck(HelloAck),
    #[serde(rename = "error")]
    Error(ProtocolError),
    #[serde(rename = "server.deploy")]
    ServerDeploy(ServerDeploy),
    #[serde(rename = "server.delete")]
    ServerDelete(ServerDelete),
    #[serde(rename = "server.power")]
    ServerPower(ServerPower),
    #[serde(rename = "server.sync")]
    ServerSync(ServerSync),
    #[serde(rename = "server.watch")]
    ServerWatch(ServerWatch),
    #[serde(rename = "server.install")]
    ServerInstall(ServerInstall),
    #[serde(rename = "server.command")]
    ServerCommand(ServerCommand),
    #[serde(rename = "server.limits")]
    ServerLimits(ServerLimits),
    #[serde(rename = "server.suspend")]
    ServerSuspend(ServerSuspend),
    #[serde(rename = "file.list")]
    FileList(FileList),
    #[serde(rename = "file.read")]
    FileRead(FileRead),
    #[serde(rename = "file.write")]
    FileWrite(FileWrite),
    #[serde(rename = "file.mkdir")]
    FileMkdir(FileMkdir),
    #[serde(rename = "file.rename")]
    FileRename(FileRename),
    #[serde(rename = "file.delete")]
    FileDelete(FileDelete),
    #[serde(rename = "file.upload")]
    FileUpload(FileUpload),
    #[serde(rename = "file.upload.chunk")]
    FileUploadChunk(FileUploadChunk),
    #[serde(rename = "backup.create")]
    BackupCreate(BackupCreate),
    #[serde(rename = "backup.restore")]
    BackupRestore(BackupRestore),
    #[serde(rename = "backup.delete")]
    BackupDelete(BackupDelete),
    #[serde(rename = "backup.list")]
    BackupList(BackupList),
    #[serde(rename = "backup.read")]
    BackupRead(BackupRead),
    #[serde(rename = "content.install")]
    ContentInstall(ContentInstall),
    #[serde(rename = "content.remove")]
    ContentRemove(ContentRemove),
    #[serde(rename = "content.toggle")]
    ContentToggle(ContentToggle),
}
