//! File-backed image attachments on existing annotations.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{App, Mode};

impl App {
    pub(super) fn begin_attach_image(&mut self) {
        let placed = self.open.store.placed();
        let Some(target) = placed.get(self.rail_cursor) else {
            return;
        };
        self.compose = super::compose::Compose::default();
        self.status = Some("image path: paste or type a local PNG/JPEG/GIF/WebP/BMP/SVG file".into());
        self.mode = Mode::AttachImage(target.annotation.id.clone());
    }

    pub(super) fn attach_image_to_annotation(&mut self, id: &str, input: &str) -> Result<bool> {
        let path = image_attachment_path(input)?;
        let media_type = image_media_type(&path).ok_or_else(|| {
            anyhow::anyhow!("expected an image file (.png, .jpg, .jpeg, .gif, .webp, .bmp or .svg)")
        })?;
        anyhow::ensure!(path.is_file(), "not a file: {}", path.display());
        let alt = path.file_name().map(|name| name.to_string_lossy().into_owned());
        let attachment = plannotator_tui_schema::LocalAttachment::image(
            path.display().to_string(),
            alt,
            Some(media_type.to_owned()),
        );
        let added = self.open.store.add_image_attachment(id, attachment)?;
        if added {
            self.mark_unsent();
            self.sync_tree_counts();
        }
        Ok(added)
    }
}

fn image_attachment_path(input: &str) -> Result<PathBuf> {
    let path = input.trim().trim_matches(['\'', '"']);
    anyhow::ensure!(!path.is_empty(), "image path is empty");
    let path = file_url_path(path).unwrap_or_else(|| PathBuf::from(path));
    path.canonicalize().with_context(|| format!("resolve image attachment {}", path.display()))
}

fn file_url_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(slash) if matches!(&rest[..slash], "" | "localhost") => &rest[slash..],
        _ => return None,
    };
    let decoded = percent_decode(path);
    #[cfg(windows)]
    let decoded = match decoded.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => {
            decoded.get(1..).unwrap_or_default().to_owned()
        }
        _ => decoded,
    };
    Some(PathBuf::from(decoded))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let decoded = (bytes.get(i) == Some(&b'%'))
            .then(|| s.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = decoded {
            out.push(byte);
            i += 3;
        } else {
            out.extend(bytes.get(i..=i).unwrap_or_default());
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn image_media_type(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("bmp") => Some("image/bmp"),
        Some("svg") => Some("image/svg+xml"),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn file_urls_and_quoted_paths_become_absolute_paths() {
        assert_eq!(
            image_attachment_path("'Cargo.toml'").expect("relative path"),
            Path::new("Cargo.toml").canonicalize().expect("canonical relative path")
        );
        let dir = super::super::tests::scratch_data_dir();
        let path = dir.join("screen shot.png");
        std::fs::write(&path, b"image").expect("image");
        assert_eq!(
            image_attachment_path(&format!("'{}'", path.display())).expect("path"),
            path.canonicalize().expect("canonical path")
        );
        #[cfg(unix)]
        assert_eq!(
            image_attachment_path(&format!("file://{}/screen%20shot.png", dir.display())).expect("path"),
            path.canonicalize().expect("canonical path")
        );
        std::fs::remove_dir_all(dir).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn parent_components_follow_symlink_targets() {
        let dir = super::super::tests::scratch_data_dir();
        let work = dir.join("work");
        let captures = dir.join("captures");
        std::fs::create_dir_all(&work).expect("work directory");
        std::fs::create_dir_all(captures.join("session")).expect("capture directory");
        std::os::unix::fs::symlink(captures.join("session"), work.join("screens")).expect("screens symlink");
        std::fs::write(work.join("shot.png"), b"wrong image").expect("decoy image");
        std::fs::write(captures.join("shot.png"), b"intended image").expect("intended image");

        let resolved = image_attachment_path(&work.join("screens/../shot.png").to_string_lossy())
            .expect("attachment path");
        assert_eq!(std::fs::read(&resolved).expect("attached image"), b"intended image");
        assert_eq!(resolved, captures.join("shot.png").canonicalize().expect("canonical path"));
        std::fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn only_known_image_extensions_get_media_types() {
        assert_eq!(image_media_type(Path::new("a.PNG")), Some("image/png"));
        assert_eq!(image_media_type(Path::new("a.jpeg")), Some("image/jpeg"));
        assert_eq!(image_media_type(Path::new("a.txt")), None);
    }
}
