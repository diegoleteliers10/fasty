use std::path::Path;
use base64::prelude::*;
use crate::ai::model::ContentPart;

pub const MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024; // 32 MB limit

/// Returns true if the path points to a PDF document.
pub fn is_pdf_path(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        ext.eq_ignore_ascii_case("pdf")
    } else {
        false
    }
}

/// Loads a PDF document from disk, enforces the 32 MB size limit, and encodes to base64.
pub fn load_and_prepare_document(path: &Path) -> Result<ContentPart, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("Failed to read metadata for '{}': {}", path.display(), e))?;

    if metadata.len() > MAX_DOCUMENT_BYTES as u64 {
        return Err(format!(
            "PDF file '{}' ({:.1} MB) exceeds maximum supported size of 32 MB",
            path.display(),
            metadata.len() as f64 / (1024.0 * 1024.0)
        ));
    }

    let bytes = std::fs::read(path)
        .map_err(|e| format!("Failed to read file '{}': {}", path.display(), e))?;

    let filename = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string());

    let base64_data = BASE64_STANDARD.encode(&bytes);

    Ok(ContentPart::Document {
        media_type: "application/pdf".to_string(),
        data: base64_data,
        name: filename,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_pdf_path() {
        assert!(is_pdf_path(Path::new("doc.pdf")));
        assert!(is_pdf_path(Path::new("/path/to/FILE.PDF")));
        assert!(!is_pdf_path(Path::new("image.png")));
        assert!(!is_pdf_path(Path::new("script.rs")));
    }
}
