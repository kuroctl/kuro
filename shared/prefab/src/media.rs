//! Mise aux dimensions convenues des images d'un prefab (ADR-0011), partagée par
//! `cargo xtask prefab-media` (fichiers du dépôt) et l'API (images déjà importées).
//!
//! L'image est recadrée au centre au bon rapport, puis réduite aux dimensions de la convention,
//! jamais agrandie. Le format est conservé : un PNG est optimisé sans perte (oxipng), un JPEG ou
//! un WebP n'est réencodé que s'il a fallu le redimensionner.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageFormat};

use crate::assets::{AssetKind, MAX_ASSET_BYTES};

/// Qualité d'un JPEG réencodé (seulement quand il est redimensionné).
const JPEG_QUALITY: u8 = 90;

/// Dimensions convenues (largeur, hauteur) par type d'image. L'icône s'affiche en 56 px au plus :
/// 64 px suffisent, et c'est la taille des icônes que publie Steam (32 ou 184 px).
pub fn target(kind: AssetKind) -> (u32, u32) {
    match kind {
        AssetKind::Icon => (64, 64),
        AssetKind::Cover => (1600, 600),
        AssetKind::Thumbnail => (600, 800),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("unreadable image: {0}")]
    Unreadable(String),
    #[error("unsupported image format {0} (PNG, JPEG or WebP)")]
    Unsupported(String),
    #[error("{0} bytes once processed, above the {MAX_ASSET_BYTES} accepted")]
    TooLarge(u64),
    #[error("encoding failed: {0}")]
    Encoding(String),
}

/// Dimensions et poids d'une image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measure {
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

/// Image retravaillée.
#[derive(Debug)]
pub struct Optimized {
    pub data: Vec<u8>,
    pub before: Measure,
    pub after: Measure,
    /// Plus petite que la convention : gardée à sa taille (jamais agrandie).
    pub small: bool,
}

/// Recadre, réduit et optimise `original` ; `None` si elle est déjà conforme et optimisée.
pub fn optimize(original: &[u8], kind: AssetKind) -> Result<Option<Optimized>, MediaError> {
    let format =
        image::guess_format(original).map_err(|e| MediaError::Unreadable(e.to_string()))?;
    let image = image::load_from_memory_with_format(original, format)
        .map_err(|e| MediaError::Unreadable(e.to_string()))?;
    let (width, height) = image.dimensions();
    let (fitted, small) = fit(&image, target(kind));
    let resized = fitted.dimensions() != (width, height);
    let encoding = |e: image::ImageError| MediaError::Encoding(e.to_string());

    let encoded = match format {
        ImageFormat::Png => optimize_png(&fitted, (!resized).then_some(original))?,
        ImageFormat::Jpeg if resized => {
            let mut out = Vec::new();
            JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
                .encode_image(&fitted)
                .map_err(encoding)?;
            out
        }
        ImageFormat::WebP if resized => {
            let mut out = Cursor::new(Vec::new());
            fitted
                .write_to(&mut out, ImageFormat::WebP)
                .map_err(encoding)?;
            out.into_inner()
        }
        // Un JPEG ou un WebP n'est réencodé (avec perte) que s'il a fallu le redimensionner.
        ImageFormat::Jpeg | ImageFormat::WebP => return Ok(None),
        other => return Err(MediaError::Unsupported(format!("{other:?}"))),
    };
    if !resized && encoded.len() >= original.len() {
        return Ok(None);
    }
    let bytes = encoded.len() as u64;
    if bytes > MAX_ASSET_BYTES {
        return Err(MediaError::TooLarge(bytes));
    }
    let (fw, fh) = fitted.dimensions();
    Ok(Some(Optimized {
        data: encoded,
        before: Measure {
            width,
            height,
            bytes: original.len() as u64,
        },
        after: Measure {
            width: fw,
            height: fh,
            bytes,
        },
        small,
    }))
}

/// Recadrage centré au rapport visé, puis réduction aux dimensions visées si l'image est plus
/// grande. Rendu : l'image et « plus petite que la convention ».
pub fn fit(image: &DynamicImage, (tw, th): (u32, u32)) -> (DynamicImage, bool) {
    let (w, h) = image.dimensions();
    // Plus grand rectangle au rapport exact tw:th (réduit, ex. 3:4) tenant dans l'image : un
    // multiple entier du rapport, pour qu'une image déjà recadrée le reste (pas d'arrondi qui
    // rognerait un pixel de plus à chaque passage).
    let divisor = gcd(tw, th);
    let (rw, rh) = (tw / divisor, th / divisor);
    let scale = (w / rw).min(h / rh).max(1);
    let (cw, ch) = ((scale * rw).min(w), (scale * rh).min(h));
    let cropped = image.crop_imm((w - cw) / 2, (h - ch) / 2, cw, ch);
    if cw > tw {
        (cropped.resize_exact(tw, th, FilterType::Lanczos3), false)
    } else {
        (cropped, cw < tw)
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// PNG optimisé sans perte. Sans redimensionnement, on part du fichier d'origine (pixels
/// identiques) ; sinon de l'image recadrée.
fn optimize_png(image: &DynamicImage, original: Option<&[u8]>) -> Result<Vec<u8>, MediaError> {
    let source = match original {
        Some(bytes) => bytes.to_vec(),
        None => {
            let mut out = Cursor::new(Vec::new());
            image
                .write_to(&mut out, ImageFormat::Png)
                .map_err(|e| MediaError::Encoding(e.to_string()))?;
            out.into_inner()
        }
    };
    let mut options = oxipng::Options::from_preset(4);
    // Métadonnées sans effet sur l'affichage retirées (profil couleur et gamma gardés).
    options.strip = oxipng::StripChunks::Safe;
    oxipng::optimize_from_memory(&source, &options).map_err(|e| MediaError::Encoding(e.to_string()))
}

#[cfg(test)]
mod tests {
    use image::RgbaImage;

    use super::*;

    fn blank(width: u32, height: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::new(width, height))
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        blank(width, height)
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn a_large_banner_is_cropped_to_its_ratio_then_reduced() {
        let (fitted, small) = fit(&blank(2000, 1000), target(AssetKind::Cover));
        assert_eq!(fitted.dimensions(), (1600, 600));
        assert!(!small);
    }

    #[test]
    fn a_small_image_is_cropped_but_never_enlarged() {
        let (fitted, small) = fit(&blank(32, 40), target(AssetKind::Icon));
        assert_eq!(fitted.dimensions(), (32, 32));
        assert!(small);
    }

    #[test]
    fn a_conforming_thumbnail_is_left_as_is() {
        let (fitted, small) = fit(&blank(600, 800), target(AssetKind::Thumbnail));
        assert_eq!(fitted.dimensions(), (600, 800));
        assert!(!small);
    }

    #[test]
    fn a_small_image_off_ratio_is_cropped_once_and_then_left_alone() {
        let (fitted, small) = fit(&blank(364, 485), target(AssetKind::Thumbnail));
        assert_eq!(fitted.dimensions(), (363, 484));
        assert!(small);
        let (again, _) = fit(&fitted, target(AssetKind::Thumbnail));
        assert_eq!(again.dimensions(), (363, 484));
    }

    #[test]
    fn an_optimized_image_is_left_alone_the_second_time() {
        let first = optimize(&png(1000, 1000), AssetKind::Icon)
            .unwrap()
            .unwrap();
        assert_eq!((first.after.width, first.after.height), (64, 64));
        assert!(optimize(&first.data, AssetKind::Icon).unwrap().is_none());
    }

    #[test]
    fn a_small_off_ratio_png_is_stable_after_one_pass() {
        let first = optimize(&png(364, 485), AssetKind::Thumbnail)
            .unwrap()
            .unwrap();
        assert!(first.small);
        assert!(
            optimize(&first.data, AssetKind::Thumbnail)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_non_image_is_refused() {
        assert!(matches!(
            optimize(b"<svg/>", AssetKind::Icon),
            Err(MediaError::Unreadable(_))
        ));
    }
}
