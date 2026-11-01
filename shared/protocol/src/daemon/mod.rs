//! Messages émis par le daemon vers l'API.

pub mod backup;
pub mod console;
pub mod content;
pub mod file;
mod heartbeat;
mod hello;
pub mod server;
pub mod sftp;

use serde::{Deserialize, Serialize};

pub use backup::{BackupChunk, BackupDone, BackupInfo, BackupListing, BackupOutcome, BackupState};
pub use console::{ConsoleLine, ConsoleOutput, ServerMetrics, ServerStats};
pub use content::{ContentDone, ContentOutcome, ExtractedPackage, MAX_DESCRIPTION_BYTES};
pub use file::{
    DeletedFile, DeletedFiles, FileContent, FileEntry, FileKind, FileListing, FileVersion,
    FileVersions, VersionChannel, VersionReason,
};
pub use heartbeat::Heartbeat;
pub use hello::{Hello, HostInfo, SftpEndpoint};
pub use server::{
    ActionResult, CertificateState, CommandResult, ProxyState, ServerDisk, ServerStateReport,
    WebStatus,
};
pub use sftp::{SftpAudit, SftpAuth, SftpLease};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum DaemonMessage {
    #[serde(rename = "hello")]
    Hello(Hello),
    #[serde(rename = "heartbeat")]
    Heartbeat(Heartbeat),
    #[serde(rename = "command.result")]
    CommandResult(CommandResult),
    #[serde(rename = "server.state")]
    ServerState(ServerStateReport),
    #[serde(rename = "console.line")]
    ConsoleLine(ConsoleOutput),
    #[serde(rename = "server.stats")]
    ServerStats(ServerStats),
    #[serde(rename = "server.metrics")]
    ServerMetrics(ServerMetrics),
    #[serde(rename = "server.disk")]
    ServerDisk(ServerDisk),
    #[serde(rename = "file.listing")]
    FileListing(FileListing),
    #[serde(rename = "file.content")]
    FileContent(FileContent),
    #[serde(rename = "file.versions")]
    FileVersions(FileVersions),
    #[serde(rename = "file.deleted_list")]
    DeletedFiles(DeletedFiles),
    #[serde(rename = "backup.done")]
    BackupDone(BackupDone),
    #[serde(rename = "backup.listing")]
    BackupListing(BackupListing),
    #[serde(rename = "backup.chunk")]
    BackupChunk(BackupChunk),
    #[serde(rename = "content.done")]
    ContentDone(ContentDone),
    #[serde(rename = "action.result")]
    ActionResult(ActionResult),
    #[serde(rename = "sftp.auth")]
    SftpAuth(SftpAuth),
    #[serde(rename = "sftp.lease")]
    SftpLease(SftpLease),
    #[serde(rename = "sftp.audit")]
    SftpAudit(SftpAudit),
}
