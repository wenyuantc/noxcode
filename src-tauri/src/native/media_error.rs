use std::fmt;

/// 稳定错误码。展示文案可以变化，调用方只比较 `code`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaError {
    pub code: &'static str,
    pub message: String,
}

impl MediaError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for MediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for MediaError {}

pub const CHUNK_CONFLICT: &str = "chunk_conflict";
pub const CHUNK_OUT_OF_ORDER: &str = "chunk_out_of_order";
pub const INVALID_CHUNK: &str = "invalid_chunk";
pub const CHUNK_TOO_LARGE: &str = "chunk_too_large";
pub const DECLARED_SIZE_EXCEEDED: &str = "declared_size_exceeded";
pub const SOURCE_CHANGED: &str = "source_changed";
pub const IMPORT_INVALID: &str = "import_invalid";
pub const IDEMPOTENCY_CONFLICT: &str = "idempotency_conflict";
pub const MEDIA_BUDGET_EXCEEDED: &str = "media_budget_exceeded";
pub const COMPONENT_MISSING: &str = "component_missing";
pub const COMPONENT_INCOMPATIBLE: &str = "component_incompatible";
pub const UNAUTHORIZED: &str = "unauthorized";
pub const ATTACHMENT_DELETING: &str = "attachment_deleting";
pub const ATTACHMENT_MISSING: &str = "attachment_missing";
pub const INTEGRITY_MISMATCH: &str = "integrity_mismatch";
pub const PATH_ESCAPE: &str = "path_escape";
pub const PREVIEW_EXPIRED: &str = "preview_expired";
pub const PREVIEW_REVOKED: &str = "preview_revoked";
pub const VIDEO_UNSUPPORTED: &str = "video_unsupported";
pub const VIDEO_INVALID: &str = "video_invalid";
pub const PDF_INVALID: &str = "pdf_invalid";
pub const PDF_ENCRYPTED: &str = "pdf_encrypted";
pub const RANGE_REQUIRED: &str = "range_required";
pub const NOT_FOUND: &str = "not_found";
pub const MAINTENANCE: &str = "maintenance";
pub const BACKUP_REJECTED: &str = "backup_rejected";
pub const INPUT_BLOCKED: &str = "input_blocked";
pub const REVISION_CONFLICT: &str = "revision_conflict";
pub const INPUT_NOT_EDITABLE: &str = "input_not_editable";
pub const INVALID_RANGE: &str = "invalid_range";
pub const STORAGE_FAILED: &str = "storage_failed";
pub const CANCELLED: &str = "cancelled";
pub const TIMEOUT: &str = "timeout";
