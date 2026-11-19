//! The wallpaper's picture: how `[wallpaper] fit` places it (`Placement`), and the
//! file decoded on a thread of its own, shrunk where it's larger than any
//! monitor needs, as an iced image.

use std::path::PathBuf;

use iced::{widget::image, ContentFit};

/// The largest a picture is kept, beyond which it's shrunk (keeping its
/// shape, never grown) before it's shown: a 4K monitor's, so iced's own
/// scaling to each monitor stays small and smooth.
pub const LARGEST: (u32, u32) = (3840, 2160);

/// `[wallpaper] fit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Fill,
    Fit,
    Stretch,
    Center,
}

impl Placement {
    /// As Pila names it; `None` for a name it doesn't (a newer Pila's).
    pub fn named(name: &str) -> Option<Placement> {
        Some(match name {
            "fill" => Placement::Fill,
            "fit" => Placement::Fit,
            "stretch" => Placement::Stretch,
            "center" => Placement::Center,
            _ => return None,
        })
    }

    /// How iced places the picture on a monitor.
    pub fn content_fit(self) -> ContentFit {
        match self {
            Placement::Fill => ContentFit::Cover,
            Placement::Fit => ContentFit::Contain,
            Placement::Stretch => ContentFit::Fill,
            Placement::Center => ContentFit::None,
        }
    }
}

/// The wallpaper Pila asks for: an image and how it fits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub image: PathBuf,
    pub fit: Placement,
}

impl Wanted {
    /// A `wallpaper` answer's or event's object; `None` for no image.
    pub fn from_pila(wallpaper: &serde_json::Value) -> Option<Wanted> {
        let image = wallpaper["image"]
            .as_str()
            .filter(|path| !path.is_empty())?;
        let fit = wallpaper["fit"]
            .as_str()
            .and_then(Placement::named)
            .unwrap_or(Placement::Fill);
        Some(Wanted {
            image: PathBuf::from(image),
            fit,
        })
    }
}

/// `size` shrunk to cover no more than `largest` needs, keeping its shape:
/// as small as it can be while still at least `largest` one way or the
/// other (so `Fill` crops from enough pixels); never grown.
pub fn shrunk(size: (u32, u32), largest: (u32, u32)) -> (u32, u32) {
    let (w, h) = (f64::from(size.0.max(1)), f64::from(size.1.max(1)));
    let scale = (f64::from(largest.0) / w)
        .max(f64::from(largest.1) / h)
        .min(1.0);
    (
        ((w * scale).round() as u32).max(1),
        ((h * scale).round() as u32).max(1),
    )
}

/// The picture of `wanted`, decoded (and shrunk, but at `Center`, shown at
/// its size) on a thread: one answer, or why it can't be shown.
pub fn load(wanted: Wanted) -> impl iced::futures::Future<Output = Result<image::Handle, String>> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("pila-wallpaper-load".into())
        .spawn(move || {
            let _ = sender.send(decode(&wanted));
        });
    let started = spawned.map_err(|err| format!("can't load the picture: {err}"));
    async move {
        started?;
        receiver
            .await
            .map_err(|_| "the picture's thread ended".to_string())?
    }
}

fn decode(wanted: &Wanted) -> Result<image::Handle, String> {
    let path = wanted.image.display();
    let picture = ::image::ImageReader::open(&wanted.image)
        .map_err(|err| format!("can't open {path}: {err}"))?
        .with_guessed_format()
        .map_err(|err| format!("can't read {path}: {err}"))?
        .decode()
        .map_err(|err| format!("can't decode {path}: {err}"))?
        .into_rgba8();
    let size = picture.dimensions();
    let (w, h) = match wanted.fit {
        Placement::Center => size,
        _ => shrunk(size, LARGEST),
    };
    let picture = if (w, h) == size {
        picture
    } else {
        ::image::imageops::resize(&picture, w, h, ::image::imageops::FilterType::Triangle)
    };
    Ok(image::Handle::from_rgba(w, h, picture.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_shrunk_to_cover_the_largest_monitor_never_grown() {
        assert_eq!(shrunk((7680, 4320), LARGEST), (3840, 2160));
        // Another shape: still covers 3840 x 2160 either way.
        assert_eq!(shrunk((6000, 4000), LARGEST), (3840, 2560));
        assert_eq!(shrunk((4000, 8000), LARGEST), (3840, 7680));
        assert_eq!(shrunk((1920, 1080), LARGEST), (1920, 1080));
        assert_eq!(shrunk((0, 0), LARGEST), (1, 1));
    }

    #[test]
    fn pila_says_the_image_and_how_it_fits() {
        let wanted = |value: serde_json::Value| Wanted::from_pila(&value);
        assert_eq!(
            wanted(serde_json::json!({ "image": "/a.png", "fit": "center" })),
            Some(Wanted {
                image: "/a.png".into(),
                fit: Placement::Center
            })
        );
        assert_eq!(
            wanted(serde_json::json!({ "image": null, "fit": "fill" })),
            None
        );
        // A fit from a newer Pila: the default one.
        assert_eq!(
            wanted(serde_json::json!({ "image": "/a.png", "fit": "tile" })).map(|w| w.fit),
            Some(Placement::Fill)
        );
    }

    #[test]
    fn a_picture_loads_shrunk_and_a_missing_one_says_why() {
        let dir = std::env::temp_dir().join(format!("pila-wallpaper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.png");
        ::image::RgbImage::from_pixel(5000, 3000, ::image::Rgb([200, 10, 10]))
            .save(&path)
            .unwrap();
        let handle = decode(&Wanted {
            image: path.clone(),
            fit: Placement::Fill,
        })
        .unwrap();
        match handle {
            image::Handle::Rgba {
                width,
                height,
                pixels,
                ..
            } => {
                assert_eq!((width, height), (3840, 2304));
                assert_eq!(&pixels[..4], [200, 10, 10, 255]);
            }
            other => panic!("not RGBA: {other:?}"),
        }
        let error = decode(&Wanted {
            image: dir.join("none.png"),
            fit: Placement::Fill,
        })
        .unwrap_err();
        assert!(error.contains("none.png"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
