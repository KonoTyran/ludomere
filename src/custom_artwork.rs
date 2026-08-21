use anyhow::{Context, Result, bail};
use image::{GenericImageView, ImageFormat, ImageReader};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtworkKind {
    Cover,
    Background,
}

impl ArtworkKind {
    fn stem(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Background => "background",
        }
    }
}

pub fn import(product_id: i64, kind: ArtworkKind, source: &Path) -> Result<PathBuf> {
    import_at(&crate::identity::custom_artwork(), product_id, kind, source)
}

pub fn reset(product_id: i64, kind: ArtworkKind) -> Result<()> {
    reset_at(&crate::identity::custom_artwork(), product_id, kind)
}

pub fn override_path(product_id: i64, kind: ArtworkKind) -> Option<PathBuf> {
    override_path_at(&crate::identity::custom_artwork(), product_id, kind)
}

pub fn apply(games: &mut [crate::domain::Game]) {
    for game in games {
        if let Some(path) = override_path(game.product_id, ArtworkKind::Cover) {
            game.artwork = Some(path);
        }
        if let Some(path) = override_path(game.product_id, ArtworkKind::Background) {
            game.detail_artwork = Some(path);
        }
        for dlc in &mut game.dlcs {
            if let Some(path) = override_path(dlc.product_id, ArtworkKind::Cover) {
                dlc.artwork = Some(path);
            }
            if let Some(path) = override_path(dlc.product_id, ArtworkKind::Background) {
                dlc.detail_artwork = Some(path);
            }
        }
    }
}

fn import_at(root: &Path, product_id: i64, kind: ArtworkKind, source: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(source).context("could not inspect selected artwork")?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        bail!("selected artwork must be a regular file");
    }
    if metadata.len() > 50 * 1024 * 1024 {
        bail!("selected artwork exceeds the 50 MiB limit");
    }
    let reader = ImageReader::open(source)
        .context("could not open selected artwork")?
        .with_guessed_format()
        .context("could not detect selected artwork format")?;
    let format = reader.format();
    if !matches!(format, Some(ImageFormat::Png | ImageFormat::Jpeg)) {
        bail!("custom artwork must be a PNG or JPEG image");
    }
    let image = reader
        .decode()
        .context("could not decode selected artwork")?;
    let (width, height) = image.dimensions();
    let (minimum_width, minimum_height) = match kind {
        ArtworkKind::Cover => (128, 128),
        ArtworkKind::Background => (640, 360),
    };
    if width < minimum_width || height < minimum_height {
        bail!(
            "selected artwork is too small; minimum dimensions are {minimum_width}×{minimum_height}"
        );
    }
    if width > 16_384 || height > 16_384 || u64::from(width) * u64::from(height) > 100_000_000 {
        bail!("selected artwork dimensions are unreasonably large");
    }
    drop(image);

    let directory = root.join(product_id.to_string());
    fs::create_dir_all(&directory)?;
    let extension = if format == Some(ImageFormat::Png) {
        "png"
    } else {
        "jpg"
    };
    let destination = directory.join(format!("{}.{}", kind.stem(), extension));
    let temporary = directory.join(format!(".{}.import", kind.stem()));
    fs::copy(source, &temporary).context("could not copy selected artwork")?;
    fs::rename(&temporary, &destination).context("could not publish selected artwork")?;
    for other in ["png", "jpg"] {
        let path = directory.join(format!("{}.{}", kind.stem(), other));
        if path != destination && path.is_file() {
            fs::remove_file(path)?;
        }
    }
    Ok(destination)
}

fn reset_at(root: &Path, product_id: i64, kind: ArtworkKind) -> Result<()> {
    let directory = root.join(product_id.to_string());
    for extension in ["png", "jpg"] {
        let path = directory.join(format!("{}.{}", kind.stem(), extension));
        if path.is_file() {
            fs::remove_file(path)?;
        }
    }
    if directory.is_dir() && directory.read_dir()?.next().is_none() {
        fs::remove_dir(directory)?;
    }
    Ok(())
}

fn override_path_at(root: &Path, product_id: i64, kind: ArtworkKind) -> Option<PathBuf> {
    ["png", "jpg"]
        .into_iter()
        .map(|extension| {
            root.join(product_id.to_string())
                .join(format!("{}.{}", kind.stem(), extension))
        })
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ludomere-custom-art-{name}-{}", std::process::id()))
    }

    #[test]
    fn imports_and_resets_valid_artwork_without_touching_source() {
        let root = temp("valid");
        let source = root.with_extension("source.png");
        image::DynamicImage::new_rgb8(640, 360)
            .save(&source)
            .unwrap();
        let imported = import_at(&root, 42, ArtworkKind::Background, &source).unwrap();
        assert!(imported.is_file());
        assert!(source.is_file());
        reset_at(&root, 42, ArtworkKind::Background).unwrap();
        assert!(override_path_at(&root, 42, ArtworkKind::Background).is_none());
        fs::remove_file(source).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn rejects_small_and_unsupported_images() {
        let root = temp("invalid");
        let small = root.with_extension("small.png");
        image::DynamicImage::new_rgb8(32, 32).save(&small).unwrap();
        assert!(import_at(&root, 42, ArtworkKind::Cover, &small).is_err());
        let text = root.with_extension("txt");
        fs::write(&text, b"not an image").unwrap();
        assert!(import_at(&root, 42, ArtworkKind::Cover, &text).is_err());
        fs::remove_file(small).unwrap();
        fs::remove_file(text).unwrap();
    }
}
