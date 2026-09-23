/// 应用侧预算。比较时包含等号；渠道更严的限制另计。
pub const MAX_ORIGINAL_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_INPUT_ATTACHMENTS: usize = 8;
pub const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_PENDING_MEDIA_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_TURN_MEDIA_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_REQUEST_BYTES: usize = 16_777_216;
pub const MAX_IMAGE_BLOCKS: usize = 8;
pub const MAX_PROCESSED_IMAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_IMAGE_EDGE: u32 = 2048;
pub const MAX_DECODE_PIXELS: u64 = 64_000_000;
pub const MAX_NATIVE_PDF_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_NATIVE_PDF_PAGES: u32 = 100;
pub const MAX_PDF_RENDER_PAGES: usize = 8;
pub const MAX_VIDEOS_PER_REQUEST: usize = 1;
pub const MAX_VIDEO_BYTES: u64 = 6 * 1024 * 1024;
pub const MAX_VIDEO_SECONDS: f64 = 60.0;
pub const MAX_VIDEO_EDGE: u32 = 1080;
pub const MAX_CHUNK_BYTES: usize = 1024 * 1024;
pub const TEXT_BYTE_BUDGET: usize = 200_000;
pub const PREVIEW_TTL_SECONDS: i64 = 5 * 60;
pub const UNREFERENCED_TTL_SECONDS: i64 = 24 * 60 * 60;
pub const PDF_WORKER_TIMEOUT_SECONDS: u64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetLimits {
    pub max_original_bytes: u64,
    pub max_input_attachments: usize,
    pub max_input_bytes: u64,
    pub max_turn_media_bytes: u64,
    pub max_request_bytes: usize,
    pub max_image_blocks: usize,
    pub max_processed_image_bytes: usize,
    pub max_image_edge: u32,
    pub max_decode_pixels: u64,
    pub max_native_pdf_bytes: u64,
    pub max_native_pdf_pages: u32,
    pub max_pdf_render_pages: usize,
    pub max_videos: usize,
    pub max_video_bytes: u64,
    pub max_video_edge: u32,
}

impl BudgetLimits {
    pub const fn production() -> Self {
        Self {
            max_original_bytes: MAX_ORIGINAL_BYTES,
            max_input_attachments: MAX_INPUT_ATTACHMENTS,
            max_input_bytes: MAX_INPUT_BYTES,
            max_turn_media_bytes: MAX_TURN_MEDIA_BYTES,
            max_request_bytes: MAX_REQUEST_BYTES,
            max_image_blocks: MAX_IMAGE_BLOCKS,
            max_processed_image_bytes: MAX_PROCESSED_IMAGE_BYTES,
            max_image_edge: MAX_IMAGE_EDGE,
            max_decode_pixels: MAX_DECODE_PIXELS,
            max_native_pdf_bytes: MAX_NATIVE_PDF_BYTES,
            max_native_pdf_pages: MAX_NATIVE_PDF_PAGES,
            max_pdf_render_pages: MAX_PDF_RENDER_PAGES,
            max_videos: MAX_VIDEOS_PER_REQUEST,
            max_video_bytes: MAX_VIDEO_BYTES,
            max_video_edge: MAX_VIDEO_EDGE,
        }
    }
}

impl Default for BudgetLimits {
    fn default() -> Self {
        Self::production()
    }
}
